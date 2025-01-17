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

fn load_words(
    path: &str,
    max_words: Option<usize>,
) -> Result<Vec<(String, String)>, Box<dyn Error>> {
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
        .take(max_words.unwrap_or(std::usize::MAX))
        .collect())
}

#[derive(Debug, Default, Serialize, PartialOrd, Ord, PartialEq, Eq, Clone, Copy)]
struct Time {
    secs: u64,
    subsec_nanos: u32,
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let ms = self.secs * 1000 + (self.subsec_nanos as u64 / 1000000);
        write!(f, "{}ms", ms)
    }
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
    config: &'a SpellerConfig,
    summary: Summary,
    results: Vec<AccuracyResult<'a>>,
    start_timestamp: Time,
    total_time: Time,
}

#[derive(Serialize, Default, Debug, Clone)]
struct Summary {
    total_words: u32,
    first_position: u32,
    top_five: u32,
    any_position: u32,
    no_suggestions: u32,
    only_wrong: u32,
    slowest_lookup: Time,
    fastest_lookup: Time,
    average_time: Time,
    average_time_95pc: Time,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let percent =
            |v: u32| -> String { format!("{:.2}%", v as f32 / self.total_words as f32 * 100f32) };

        write!(
            f,
            "[#1] {} [^5] {} [any] {} [none] {} [wrong] {} [fast] {} [slow] {}",
            percent(self.first_position),
            percent(self.top_five),
            percent(self.any_position),
            percent(self.no_suggestions),
            percent(self.only_wrong),
            self.fastest_lookup,
            self.slowest_lookup
        )
    }
}

impl Summary {
    fn new<'a>(results: &[AccuracyResult<'a>]) -> Summary {
        let mut summary = Summary::default();

        results.iter().for_each(|result| {
            summary.total_words += 1;

            if let Some(position) = result.position {
                summary.any_position += 1;

                if position == 0 {
                    summary.first_position += 1;
                }

                if position < 5 {
                    summary.top_five += 1;
                }
            } else if result.suggestions.len() == 0 {
                summary.no_suggestions += 1;
            } else {
                summary.only_wrong += 1;
            }
        });

        summary.slowest_lookup = results
            .iter()
            .max_by(|x, y| x.time.cmp(&y.time))
            .unwrap()
            .time
            .clone();
        summary.fastest_lookup = results
            .iter()
            .min_by(|x, y| x.time.cmp(&y.time))
            .unwrap()
            .time
            .clone();

        summary
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("divvunspell-accuracy")
        .setting(AppSettings::ArgRequiredElseHelp)
        .version(env!("CARGO_PKG_VERSION"))
        .author("Brendan Molloy <brendan@bbqsrc.net>")
        .about("Accuracy testing for DivvunSpell.")
        .arg(
            Arg::with_name("config")
                .short("c")
                .takes_value(true)
                .help("Provide JSON config file to override test defaults"),
        )
        .arg(
            Arg::with_name("words")
                .value_name("WORDS")
                .help("The 'input -> expected' list in tab-delimited value file (TSV)"),
        )
        .arg(
            Arg::with_name("zhfst")
                .value_name("ZHFST")
                .help("Use the given ZHFST file"),
        )
        .arg(
            Arg::with_name("json-output")
                .short("o")
                .value_name("JSON-OUTPUT")
                .help("The file path for the JSON report output"),
        )
        .arg(
            Arg::with_name("max-words")
                .short("w")
                .takes_value(true)
                .help("Truncate typos list to max number of words specified"),
        )
        .get_matches();

    let cfg: SpellerConfig = match matches.value_of("config") {
        Some(path) => {
            let file = std::fs::File::open(path)?;
            serde_json::from_reader(file)?
        }
        None => CFG.clone(),
    };

    let archive = match matches.value_of("zhfst") {
        Some(path) => SpellerArchive::new(path)?,
        None => {
            eprintln!("No ZHFST found for given path; aborting.");
            std::process::exit(1);
        }
    };

    let words = match matches.value_of("words") {
        Some(path) => load_words(
            path,
            matches
                .value_of("max-words")
                .and_then(|x| x.parse::<usize>().ok()),
        )?,
        None => {
            eprintln!("No word list for given path; aborting.");
            std::process::exit(1);
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
            let suggestions = archive.speller().suggest_with_config(&input, &cfg);
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

    let summary = Summary::new(&results);
    println!("{}", summary);

    if let Some(path) = matches.value_of("json-output") {
        let output = std::fs::File::create(path)?;
        let report = Report {
            metadata: archive.metadata(),
            config: &cfg,
            summary,
            results,
            start_timestamp,
            total_time,
        };
        println!("Writing JSON report…");
        serde_json::to_writer_pretty(output, &report)?;
    };

    println!("Done!");
    Ok(())
}
