mod monitor;

use monitor::{is_skippable, parse_line, Processor};
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process;

fn main() {
    if let Err(error) = run() {
        eprintln!("machine-monitor: {error}");
        process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let input = args.next();

    if matches!(input.as_deref(), Some("-h" | "--help")) {
        println!("usage: machine-monitor-mvp [FILE]\n\nReads CSV from FILE or stdin.");
        return Ok(());
    }
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected at most one input file",
        ));
    }

    let source: Box<dyn BufRead> = match input {
        Some(path) => Box::new(BufReader::new(File::open(path)?)),
        None => Box::new(BufReader::new(io::stdin().lock())),
    };

    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut processor = Processor::new();
    let mut unparsable = 0_u64;

    writeln!(output, "timestamp,state")?;

    for (index, line) in source.lines().enumerate() {
        let line = line?;
        if is_skippable(&line) {
            continue;
        }

        match parse_line(&line) {
            Ok(reading) => write_states(&mut output, processor.push(reading))?,
            Err(error) => {
                unparsable += 1;
                eprintln!("line {} skipped: {error}", index + 1);
            }
        }
    }

    write_states(&mut output, processor.flush())?;
    output.flush()?;

    let stats = processor.stats();
    eprintln!(
        "{} duplicates, {} too late, {} unparsable",
        stats.duplicates, stats.too_late, unparsable
    );
    Ok(())
}

fn write_states(output: &mut impl Write, states: Vec<monitor::Output>) -> io::Result<()> {
    for item in states {
        writeln!(output, "{},{}", item.timestamp, item.state)?;
    }
    Ok(())
}
