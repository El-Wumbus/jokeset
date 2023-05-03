#![feature(async_closure)]
use std::{
    mem,
    num::{NonZeroU16, NonZeroU64},
    path::{Path, PathBuf},
    time::Duration,
};

use csv::StringRecord;
use csv_async as csv;
use indicatif::ProgressStyle;
use serde::{Deserialize, Serialize};
use structopt::StructOpt;
use tokio::{
    fs::File,
    io::{BufReader, BufWriter},
    task, time,
};
use tokio_stream::StreamExt;
use tracing::{info_span, Span, metadata::LevelFilter, event, Level};
use tracing_indicatif::{
    filter::{hide_indicatif_span_fields, IndicatifFilter},
    span_ext::IndicatifSpanExt,
    IndicatifLayer,
};
use tracing_subscriber::{
    fmt::format::DefaultFields,
    layer:: SubscriberExt,
    util::SubscriberInitExt,
    Layer, EnvFilter,
};

// mod reddit;
mod error;
use error::Error;
mod wocka;
use wocka::WockaJoke;

const WOCKA_JOKE_PATH: &str = "wocka.csv";
const WOCKA_JOKE_JSON: &str = "wocka.json";

#[derive(Debug, Clone, PartialEq, StructOpt)]
enum Options {
    /// Scrape jokes from wocka.com
    Scrape {
        /// Output file in CSV format
        #[structopt(long, short, default_value = WOCKA_JOKE_PATH, name = "FILE")]
        output: PathBuf,

        /// Jokes are scraped in batches, this is the size of a batch. Large batches may lead to an IP block.
        #[structopt(short, long, default_value = "50")]
        tasks: NonZeroU16,

        #[structopt(short, long)]
        resume: Option<NonZeroU64>,

        /// Length limit, in characters, of a joke body.
        #[structopt(short, long, name = "COUNT")]
        length_limit: Option<usize>,
    },
    /// Count rows/jokes in a CSV file
    Count {
        #[structopt(default_value = WOCKA_JOKE_PATH, name = "FILE")]
        input: PathBuf,
    },

    /// Convert the Joke CSV to JSON
    Json {
        #[structopt(default_value = WOCKA_JOKE_PATH, name = "IN")]
        input: PathBuf,

        /// Output file in CSV format
        #[structopt(long, short, default_value = WOCKA_JOKE_JSON, name = "OUT")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let options = Options::from_args();

    let indicatif_layer = IndicatifLayer::new()
        .with_span_field_formatter(hide_indicatif_span_fields(DefaultFields::new()));

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .parse("")?;
    
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(indicatif_layer.get_stderr_writer())
                .with_filter(filter),
        )
        .with(indicatif_layer.with_filter(IndicatifFilter::new(true)))
        .init();

    match options {
        Options::Scrape {
            output,
            tasks,
            resume: _,
            length_limit,
        } => {
            scrape(&output, tasks.into(), length_limit).await?;
        }
        Options::Count { input } => {
            let joke_file_reader = BufReader::new(File::open(&input).await?);
            let mut csv_deseraializer = csv::AsyncDeserializer::from_reader(joke_file_reader);
            let mut record = StringRecord::new();
            let mut rows = 0;
            while csv_deseraializer.read_record(&mut record).await? {
                rows += 1;
            }
            println!("\"{}\" has {rows} jokes.", input.display());
        }
        Options::Json { input, output } => {
            let joke_file_reader = BufReader::new(File::open(&input).await?);
            let mut csv_deseraializer = csv::AsyncDeserializer::from_reader(joke_file_reader);
            let mut csv_stream = csv_deseraializer.deserialize::<Joke>();
            let mut jokes: Vec<Joke> = Vec::new();
            while let Some(joke) = csv_stream.next().await {
                let joke = joke?;
                jokes.push(joke);
            }
            let joke_file_json_writer =
                std::io::BufWriter::new(File::create(&output).await?.into_std().await);
            task::spawn_blocking(move || {
                serde_json::to_writer_pretty(joke_file_json_writer, &jokes)
            })
            .await??;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct Joke {
    title: String,
    body: String,
    footer: String,
}

impl From<WockaJoke> for Joke {
    fn from(value: WockaJoke) -> Self {
        Self {
            footer: format!(
                "Source: wocka.com, Category: {}, ID: {}",
                value.category, value.id
            ),
            body: value.body,
            title: value.title,
        }
    }
}

async fn scrape(output: &Path, joke_tasks: u16, length_limit: Option<usize>) -> Result<(), Error> {
    let joke_file_writer = BufWriter::new(File::create(output).await?);
    let mut csv_serializer = csv::AsyncSerializer::from_writer(joke_file_writer);
    let mut offset: u64 = 0;
    let mut errors = 0;
    println!("Starting...");
    let header_span = info_span!("header");
    header_span.pb_set_style(&ProgressStyle::default_bar());
    header_span.pb_set_length(joke_tasks as u64);
    let header_span_enter = header_span.enter();
    loop {
        Span::current().pb_set_position(0);
        // We collect so we consume the iterator and run the map.
        let tasks = (1_u16..=joke_tasks)
            .map(|i| {
                task::spawn(async move {
                    wocka::extract_joke(NonZeroU64::new(i as u64 + offset).unwrap()).await
                })
            })
            .collect::<Vec<_>>();


        for task in tasks {
            Span::current().pb_inc(1);
            match task.await.unwrap() {
                
                Ok(x) => {
                    let joke: Joke = x.into();
                    if let Some(length_limit) = length_limit {
                        if joke.body.len() > length_limit {
                            continue;
                        }
                    }
                    csv_serializer.serialize(&joke).await.unwrap();
                    errors = 0;
                }
                Err(Error::Unhandled(_)) => {
                    errors += 1;
                }
                Err(e) => {
                    event!(Level::ERROR, "{e}");
                }
            };
        }
        offset += joke_tasks as u64;
        event!(Level::INFO, "{offset} Joke pages visited.");
        // Only stop if we've failed 1500 times in a row, that way we've likey reached the end.
        if errors > 1500 {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    mem::drop(header_span_enter);
    Ok(())
}
