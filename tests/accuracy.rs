use std::error::Error;
use std::io;
use std::process;
use std::time::{SystemTime, Instant};

use divvunspell::archive::SpellerArchive;
use divvunspell::speller::suggestion::Suggestion;
use divvunspell::speller::{Speller, SpellerConfig};
use divvunspell::transducer::chunk::{ChfstBundle, ChfstTransducer};
use divvunspell::transducer::HfstTransducer;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde_derive::Serialize;
use indicatif::{ProgressBar, ProgressStyle, ParProgressBarIter, ParallelProgressIterator};

static CFG: SpellerConfig = SpellerConfig {
    max_weight: Some(50000.0),
    n_best: Some(10),
    beam: None,
    pool_max: 128,
    pool_start: 128,
    seen_node_sample_rate: 15,
    with_caps: true,
};

fn load_words() -> Vec<(String, String)> {
    let mut rdr = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .delimiter(b'\t')
        .has_headers(false)
        .flexible(true)
        .from_path("./typos.txt")
        .expect("typos");

    rdr.records()
        .filter_map(Result::ok)
        .filter_map(|r| r.get(0).and_then(|x| r.get(1).map(|y| (x.to_string(), y.to_string()))))
        .collect()
}

#[derive(Debug, Serialize)]
struct Time {
    secs: u64,
    subsec_nanos: u32
}

#[derive(Debug, Serialize)]
struct AccuracyResult {
    input: String,
    expected: String,
    suggestions: Vec<Suggestion>,
    position: Option<usize>,
    time: Time
}

#[derive(Debug, Serialize)]
struct Report<'a> {
    metadata: &'a divvunspell::archive::meta::SpellerMetadata,
    config: &'static SpellerConfig,
    results: Vec<AccuracyResult>,
    start_timestamp: Time,
    total_time: Time
}

fn main() -> Result<(), Box<dyn Error>> {
    let archive = SpellerArchive::new("./se-stored-20190817.zhfst").expect("file");
    let words = load_words();
    let pb = ProgressBar::new(words.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{pos}/{len} [{percent}%] {wide_bar} {elapsed_precise}"));

    // println!("Getting system profile…");
    // let cpuinfo = cpuid::identify().expect("no CPU identity could be found");


    println!("Running accuracy test…");
    let start_time = Instant::now();
    let results = words.par_iter().progress_with(pb).map(|(input, expected)| {
        let now = Instant::now();
        let suggestions = archive.speller().suggest_with_config(&input, &CFG);
        let now = now.elapsed();

        let time = Time { secs: now.as_secs(), subsec_nanos: now.subsec_nanos() };

        let position = suggestions.iter().position(|x| x.value == expected);

        AccuracyResult {
            input: input.to_string(),
            expected: expected.to_string(),
            time,
            suggestions,
            position
        }
    }).collect::<Vec<_>>();

    let now = start_time.elapsed();
    let total_time = Time { secs: now.as_secs(), subsec_nanos: now.subsec_nanos() };
    let now_date = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
    let start_timestamp = Time { secs: now_date.as_secs(), subsec_nanos: now_date.subsec_nanos() };

    let report = Report {
        metadata: archive.metadata(),
        config: &CFG,
        results,
        start_timestamp,
        total_time
    };

    println!("Writing JSON report…");
    serde_json::to_writer_pretty(std::fs::File::create("./report.json")?, &report)?;

    println!("Done!");
    Ok(())
}