use std::error::Error;
use std::time::{Instant, SystemTime};

use clap::{App, AppSettings, Arg};
use divvunspell::archive::SpellerArchive;
use divvunspell::speller::suggestion::Suggestion;
use divvunspell::speller::SpellerConfig;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde_derive::Serialize;

static CFG: SpellerConfig = SpellerConfig {
    max_weight: Some(50000.0),
    n_best: Some(10),
    beam: None,
    pool_max: 128,
    pool_start: 128,
    seen_node_sample_rate: 15,
    with_caps: true,
};

fn load_words(path: &str) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .delimiter(b'\t')
        .has_headers(false)
        .flexible(true)
        .from_path(path)?;

    Ok(rdr
        .records()
        .filter_map(Result::ok)
        .filter_map(|r| {
            r.get(0)
                .and_then(|x| r.get(1).map(|y| (x.to_string(), y.to_string())))
        })
        .collect())
}

#[derive(Debug, Serialize)]
struct Time {
    secs: u64,
    subsec_nanos: u32,
}

#[derive(Debug, Serialize)]
struct AccuracyResult<'a> {
    input: &'a str,
    expected: &'a str,
    suggestions: Vec<Suggestion>,
    position: Option<usize>,
    time: Time,
}

#[derive(Debug, Serialize)]
struct Report<'a> {
    metadata: &'a divvunspell::archive::meta::SpellerMetadata,
    config: &'static SpellerConfig,
    results: Vec<AccuracyResult<'a>>,
    start_timestamp: Time,
    total_time: Time,
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("divvunspell-accuracy")
        .setting(AppSettings::ArgRequiredElseHelp)
        .version(env!("CARGO_PKG_VERSION"))
        .author("Brendan Molloy <brendan@bbqsrc.net>")
        .about("Accuracy testing for DivvunSpell.")
        .arg(
            Arg::with_name("words")
                .value_name("WORDS")
                // .required(true)
                .help("The 'input -> expected' list in tab-delimited value file (TSV)"),
        )
        .arg(
            Arg::with_name("zhfst")
                .value_name("ZHFST")
                // .required(true)
                .help("Use the given ZHFST file"), // .takes_value(true),
        )
        .arg(
            Arg::with_name("json-output")
                .value_name("JSON-OUTPUT")
                .help("The file path for the JSON report output"),
        )
        .get_matches();

    let archive = match matches.value_of("zhfst") {
        Some(path) => SpellerArchive::new(path)?,
        None => {
            eprintln!("No ZHFST found for given path; aborting.");
            std::process::exit(1);
        }
    };

    let words = match matches.value_of("words") {
        Some(path) => load_words(path)?,
        None => {
            eprintln!("No word list for given path; aborting.");
            std::process::exit(1);
        }
    };

    let output = match matches.value_of("json-output") {
        Some(path) => std::fs::File::create(path)?,
        None => {
            eprintln!("No JSON output path; aborting.");
            std::process::exit(1)
        }
    };

    let pb = ProgressBar::new(words.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{pos}/{len} [{percent}%] {wide_bar} {elapsed_precise}"),
    );

    let start_time = Instant::now();
    let results = words
        .par_iter()
        .progress_with(pb)
        .map(|(input, expected)| {
            let now = Instant::now();
            let suggestions = archive.speller().suggest_with_config(&input, &CFG);
            let now = now.elapsed();

            let time = Time {
                secs: now.as_secs(),
                subsec_nanos: now.subsec_nanos(),
            };

            let position = suggestions.iter().position(|x| x.value == expected);

            AccuracyResult {
                input,
                expected,
                time,
                suggestions,
                position,
            }
        })
        .collect::<Vec<_>>();

    let now = start_time.elapsed();
    let total_time = Time {
        secs: now.as_secs(),
        subsec_nanos: now.subsec_nanos(),
    };
    let now_date = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let start_timestamp = Time {
        secs: now_date.as_secs(),
        subsec_nanos: now_date.subsec_nanos(),
    };

    let report = Report {
        metadata: archive.metadata(),
        config: &CFG,
        results,
        start_timestamp,
        total_time,
    };

    println!("Writing JSON report…");
    serde_json::to_writer_pretty(output, &report)?;

    println!("Done!");
    Ok(())
}
