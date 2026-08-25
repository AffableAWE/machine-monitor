use std::collections::HashSet;
use std::fmt;

type Timestamp = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sensor {
    Temperature,
    Vibration,
    Power,
}

impl Sensor {
    fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "temperature" | "temp" => Some(Self::Temperature),
            "vibration" | "vib" => Some(Self::Vibration),
            "power" | "pwr" => Some(Self::Power),
            _ => None,
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::Temperature => 0,
            Self::Vibration => 1,
            Self::Power => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reading {
    id: String,
    timestamp: Timestamp,
    sensor: Sensor,
    value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineState {
    Idle,
    Starting,
    Running,
    Warning,
    Unknown,
}

impl fmt::Display for MachineState {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Idle => "IDLE",
            Self::Starting => "STARTING",
            Self::Running => "RUNNING",
            Self::Warning => "WARNING",
            Self::Unknown => "UNKNOWN",
        };
        output.write_str(label)
    }
}

#[derive(Debug, Clone, Copy)]
struct Config {
    allowed_lateness: i64,
    stale_after: i64,
    idle_below: f64,
    running_at: f64,
    warning_temperature: f64,
    warning_vibration: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allowed_lateness: 5,
            stale_after: 30,
            idle_below: 0.5,
            running_at: 5.0,
            warning_temperature: 80.0,
            warning_vibration: 7.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    timestamp: Timestamp,
    value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub timestamp: Timestamp,
    pub state: MachineState,
}

#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub duplicates: u64,
    pub too_late: u64,
}

/// Accepts readings in arrival order and emits states in event-time order.
pub struct Processor {
    config: Config,
    seen_ids: HashSet<String>,
    buffer: Vec<Reading>,
    highest_timestamp: Option<Timestamp>,
    released_through: Option<Timestamp>,
    latest: [Option<Sample>; 3],
    duplicates: u64,
    too_late: u64,
}

impl Processor {
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    fn with_config(config: Config) -> Self {
        Self {
            config,
            seen_ids: HashSet::new(),
            buffer: Vec::new(),
            highest_timestamp: None,
            released_through: None,
            latest: [None; 3],
            duplicates: 0,
            too_late: 0,
        }
    }

    pub fn push(&mut self, reading: Reading) -> Vec<Output> {
        if !self.seen_ids.insert(reading.id.clone()) {
            self.duplicates += 1;
            return Vec::new();
        }

        if self
            .released_through
            .is_some_and(|watermark| reading.timestamp <= watermark)
        {
            self.too_late += 1;
            return Vec::new();
        }

        let highest = self
            .highest_timestamp
            .map_or(reading.timestamp, |old| old.max(reading.timestamp));
        self.highest_timestamp = Some(highest);

        self.buffer.push(reading);
        self.buffer.sort_by_key(|item| item.timestamp);

        let watermark = highest.saturating_sub(self.config.allowed_lateness);
        self.release_through(watermark)
    }

    pub fn flush(&mut self) -> Vec<Output> {
        match self.highest_timestamp {
            Some(highest) => self.release_through(highest),
            None => Vec::new(),
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            duplicates: self.duplicates,
            too_late: self.too_late,
        }
    }

    fn release_through(&mut self, watermark: Timestamp) -> Vec<Output> {
        if self
            .released_through
            .is_some_and(|previous| watermark <= previous)
        {
            return Vec::new();
        }

        let split = self
            .buffer
            .partition_point(|reading| reading.timestamp <= watermark);
        let ready: Vec<Reading> = self.buffer.drain(..split).collect();
        self.released_through = Some(watermark);
        self.process(ready)
    }

    fn process(&mut self, readings: Vec<Reading>) -> Vec<Output> {
        let mut outputs = Vec::new();
        let mut index = 0;

        while index < readings.len() {
            let timestamp = readings[index].timestamp;

            while index < readings.len() && readings[index].timestamp == timestamp {
                self.observe(&readings[index]);
                index += 1;
            }

            outputs.push(Output {
                timestamp,
                state: self.evaluate(timestamp),
            });
        }

        outputs
    }

    fn observe(&mut self, reading: &Reading) {
        let slot = reading.sensor.slot();
        let should_replace = self.latest[slot]
            .map(|old| reading.timestamp >= old.timestamp)
            .unwrap_or(true);

        if should_replace {
            self.latest[slot] = Some(Sample {
                timestamp: reading.timestamp,
                value: reading.value,
            });
        }
    }

    fn evaluate(&self, now: Timestamp) -> MachineState {
        let power = self.fresh_value(Sensor::Power, now);
        let temperature = self.fresh_value(Sensor::Temperature, now);
        let vibration = self.fresh_value(Sensor::Vibration, now);

        let Some(power) = power else {
            return MachineState::Unknown;
        };

        if temperature.is_some_and(|value| value >= self.config.warning_temperature)
            || vibration.is_some_and(|value| value >= self.config.warning_vibration)
        {
            return MachineState::Warning;
        }

        if power < self.config.idle_below {
            return MachineState::Idle;
        }

        if temperature.is_none() || vibration.is_none() {
            return MachineState::Unknown;
        }

        if power >= self.config.running_at {
            MachineState::Running
        } else {
            MachineState::Starting
        }
    }

    fn fresh_value(&self, sensor: Sensor, now: Timestamp) -> Option<f64> {
        self.latest[sensor.slot()].and_then(|sample| {
            let age = now.saturating_sub(sample.timestamp);
            (age <= self.config.stale_after).then_some(sample.value)
        })
    }
}

pub fn parse_line(line: &str) -> Result<Reading, String> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() != 4 {
        return Err(format!("expected 4 fields, got {}", fields.len()));
    }
    if fields[0].is_empty() {
        return Err("message id is empty".to_string());
    }

    let timestamp = fields[1]
        .parse::<Timestamp>()
        .map_err(|_| format!("invalid timestamp {:?}", fields[1]))?;
    let sensor =
        Sensor::parse(fields[2]).ok_or_else(|| format!("unknown sensor {:?}", fields[2]))?;
    let value = fields[3]
        .parse::<f64>()
        .map_err(|_| format!("invalid value {:?}", fields[3]))?;

    if !value.is_finite() {
        return Err(format!("value must be finite, got {:?}", fields[3]));
    }

    Ok(Reading {
        id: fields[0].to_string(),
        timestamp,
        sensor,
        value,
    })
}

pub fn is_skippable(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.starts_with('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(id: &str, timestamp: Timestamp, sensor: Sensor, value: f64) -> Reading {
        Reading {
            id: id.to_string(),
            timestamp,
            sensor,
            value,
        }
    }

    fn state_for(power: f64, temperature: Option<f64>, vibration: Option<f64>) -> MachineState {
        let mut processor = Processor::new();
        processor.observe(&reading("p", 100, Sensor::Power, power));
        if let Some(value) = temperature {
            processor.observe(&reading("t", 100, Sensor::Temperature, value));
        }
        if let Some(value) = vibration {
            processor.observe(&reading("v", 100, Sensor::Vibration, value));
        }
        processor.evaluate(100)
    }

    #[test]
    fn parses_a_valid_reading() {
        let item = parse_line("m1, 1000, TEMP, 42.5").unwrap();
        assert_eq!(item.id, "m1");
        assert_eq!(item.timestamp, 1000);
        assert_eq!(item.sensor, Sensor::Temperature);
        assert_eq!(item.value, 42.5);
    }

    #[test]
    fn rejects_malformed_readings() {
        assert!(parse_line("m1,1000,power").is_err());
        assert!(parse_line("m1,later,power,2.0").is_err());
        assert!(parse_line("m1,1000,humidity,2.0").is_err());
        assert!(parse_line("m1,1000,power,NaN").is_err());
    }

    #[test]
    fn removes_duplicate_message_ids() {
        let mut processor = Processor::new();
        let item = reading("same", 100, Sensor::Power, 1.0);
        processor.push(item.clone());
        processor.push(item);
        assert_eq!(processor.stats().duplicates, 1);
    }

    #[test]
    fn restores_event_time_order_within_the_lateness_window() {
        let mut processor = Processor::new();
        processor.push(reading("a", 100, Sensor::Power, 1.0));
        processor.push(reading("b", 104, Sensor::Power, 1.0));
        processor.push(reading("c", 102, Sensor::Power, 1.0));
        let output = processor.push(reading("d", 108, Sensor::Power, 1.0));
        let times: Vec<Timestamp> = output.iter().map(|item| item.timestamp).collect();
        assert_eq!(times, vec![100, 102]);
    }

    #[test]
    fn drops_messages_older_than_the_watermark() {
        let mut processor = Processor::new();
        processor.push(reading("a", 100, Sensor::Power, 1.0));
        processor.push(reading("b", 110, Sensor::Power, 1.0));
        processor.push(reading("late", 101, Sensor::Power, 1.0));
        assert_eq!(processor.stats().too_late, 1);
    }

    #[test]
    fn classifies_the_normal_states() {
        assert_eq!(state_for(0.2, Some(25.0), Some(0.2)), MachineState::Idle);
        assert_eq!(
            state_for(2.0, Some(30.0), Some(1.0)),
            MachineState::Starting
        );
        assert_eq!(state_for(6.0, Some(40.0), Some(2.0)), MachineState::Running);
    }

    #[test]
    fn warning_has_priority() {
        assert_eq!(state_for(0.2, Some(90.0), None), MachineState::Warning);
        assert_eq!(state_for(6.0, None, Some(8.0)), MachineState::Warning);
    }

    #[test]
    fn missing_or_stale_data_becomes_unknown() {
        assert_eq!(state_for(6.0, None, Some(1.0)), MachineState::Unknown);

        let mut processor = Processor::new();
        processor.observe(&reading("p", 100, Sensor::Power, 6.0));
        assert_eq!(processor.evaluate(131), MachineState::Unknown);
    }

    #[test]
    fn emits_once_for_each_distinct_timestamp() {
        let mut processor = Processor::new();
        for (id, sensor, value) in [
            ("p1", Sensor::Power, 0.2),
            ("t1", Sensor::Temperature, 25.0),
            ("v1", Sensor::Vibration, 0.1),
        ] {
            processor.push(reading(id, 100, sensor, value));
        }

        let output = processor.flush();
        assert_eq!(
            output,
            vec![Output {
                timestamp: 100,
                state: MachineState::Idle
            }]
        );
    }
}
