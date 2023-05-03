#![feature(async_closure)]
use std::{
    num::{NonZeroU64, NonZeroUsize, NonZeroU16},
    path::PathBuf,
    process::exit,
    sync::Arc,
    time::Duration,
};

use csv::StringRecord;
use csv_async as csv;
use indicatif::{ProgressIterator, ProgressStyle};
use serde::{Deserialize, Serialize};
use structopt::StructOpt;
use tokio::{
    fs::{self, File},
    io::{BufReader, BufWriter, AsyncWriteExt},
    task,
};

// mod reddit;
mod error;
mod wocka;
use error::Error;

pub const PROGRAM_NAME: &str = env!("CARGO_PKG_NAME");
pub const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
const OS: &str = std::env::consts::OS;

const REDDIT_JOKE_PATH: &str = "../reddit_jokes.json";
const STUPIDSTUFF_JOKE_PATH: &str = "../stupidstuff.json";
const WOCKA_JOKE_PATH: &str = "wocka.csv";
const OUTPUT_JOKE_PATH: &str = "jokes_filtered.csv";
const MINIMUM_REDDIT_UPVOTES: isize = 32;
const MINIMUM_STUPIDSTUFF_RATING: f64 = 3.5;

#[derive(Debug, Clone, PartialEq, StructOpt)]
enum Options {
    Scrape {
        #[structopt(long, short, default_value = WOCKA_JOKE_PATH, name = "FILE")]
        output: PathBuf,

        #[structopt(long)]
        no_wocka: bool,

        /// Jokes are scraped in batches, this is the size of a batch.
        #[structopt(short, long, default_value = "50")]
        tasks: NonZeroU16,
    },
    Count {
        #[structopt(long, short, default_value = WOCKA_JOKE_PATH, name = "FILE")]
        input: PathBuf,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct RedditJoke {
    id: String,
    score: isize,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct WockaJoke {
    title: String,
    body: String,
    id: NonZeroU64,
    category: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct StupidstuffJoke {
    category: String,
    body: String,
    id: NonZeroU64,
    rating: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct Joke {
    footer: String,
    body: String,
    title: String,
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

impl From<RedditJoke> for Joke {
    fn from(value: RedditJoke) -> Self {
        Self {
            footer: format!(
                "Source: reddit.com/r/jokes, Rating: {}, ID: {}",
                value.score, value.id
            ),
            body: value.body,
            title: value.title,
        }
    }
}

impl From<StupidstuffJoke> for Joke {
    fn from(value: StupidstuffJoke) -> Self {
        Self {
            footer: format!(
                "Source: stupidstuff.org, Category: {}, Rating: {}, ID: {}",
                value.category, value.rating, value.id
            ),
            body: value.body,
            title: "".into(),
        }
    }
}

async fn scrape(output: PathBuf, no_wocka: bool, joke_tasks: u16) -> Result<(), Error> {
    let joke_file_writer = BufWriter::new(File::create(output).await?);
    let mut csv_serializer = csv::AsyncSerializer::from_writer(joke_file_writer);
    let mut offset: u64 = 0;
    let mut errors = 0;
    println!("Starting...");
    loop {
        let tasks = (1 as u16..=joke_tasks)
            .into_iter()
            .map(|i| {
                task::spawn(async move {
                    wocka::extract_joke(NonZeroU64::new(i as u64 + offset).unwrap()).await
                })
            })
            .collect::<Vec<_>>()
            .into_iter();

        for task in tasks.progress() {
            match task.await.unwrap() {
                Ok(x) => {
                    let joke: Joke = x.into();
                    // let json = task::spawn_blocking(move|| serde_json::to_string(&joke)).await??;
                    csv_serializer.serialize(&joke).await.unwrap();
                    // let x = joke_file_writer.write_all(json.as_bytes()).await?;
                    errors = 0;
                }
                Err(e) => {
                    errors += 1;
                }
            };
            // Sleep for a bit to maybe not get blocked.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if errors > 2000 {
            break;
        }
        offset += joke_tasks as u64;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let options = Options::from_args();

    match options {
        Options::Scrape {
            output,
            no_wocka,
            tasks,
        } => scrape(output, no_wocka, tasks.into()).await?,
        Options::Count { input } => {
            let joke_file_reader = BufReader::new(File::open(&input).await?);
            let mut csv_deseraializer = csv::AsyncDeserializer::from_reader(joke_file_reader);
            let mut record = StringRecord::new();
            let mut rows = 0;
            while csv_deseraializer.read_record(&mut record).await? {
                rows +=1;
            }
            println!("\"{}\" has {rows} jokes.", input.display());
        }
    }

    // println!("Filtering...");
    // let mut jokes = filter_reddit().await;
    // jokes.append(&mut filter_stupidstuff().await?);
    // jokes.append(&mut filter_wocka().await);
    // let joke_file_writer = BufWriter::new(File::create(OUTPUT_JOKE_PATH).await?);
    // let mut csv_serializer = csv::AsyncSerializer::from_writer(joke_file_writer);
    // println!("Saving...");
    // for joke in jokes.iter().progress() {
    //     csv_serializer.serialize(joke).await?;
    // }

    Ok(())
}

async fn filter_stupidstuff() -> Result<Vec<Joke>, Error> {
    let jokes_data = fs::read(STUPIDSTUFF_JOKE_PATH).await?;
    let jokes = task::spawn_blocking(move || {
        serde_json::from_slice::<Vec<StupidstuffJoke>>(&jokes_data).unwrap()
    })
    .await
    .unwrap();
    // Filter out the following jokes:
    //
    // - Jokes with less than `MINIMUM_REDDIT_UPVOTES` score
    Ok(tokio_rayon::spawn(move || {
        jokes
            .into_iter()
            .progress()
            .filter(|joke| joke.rating >= MINIMUM_STUPIDSTUFF_RATING)
            .map(|x| {
                let x: Joke = x.into();
                x
            })
            .collect::<Vec<_>>()
    })
    .await)
}

async fn filter_reddit() -> Vec<Joke> {
    let reddit_jokes_data = fs::read(REDDIT_JOKE_PATH).await.unwrap();
    let reddit_jokes = task::spawn_blocking(move || {
        serde_json::from_slice::<Vec<RedditJoke>>(&reddit_jokes_data).unwrap()
    })
    .await
    .unwrap();

    tokio_rayon::spawn(move || {
        // Filter out the following jokes:
        //
        // - Jokes with less than `MINIMUM_REDDIT_UPVOTES` score
        reddit_jokes
            .into_iter()
            .progress()
            .filter(|joke| joke.score >= MINIMUM_REDDIT_UPVOTES)
            .map(|x| {
                let x: Joke = x.into();
                x
            })
            .collect::<Vec<_>>()
    })
    .await
}

async fn filter_wocka() -> Vec<Joke> {
    let jokes_data = fs::read(WOCKA_JOKE_PATH).await.unwrap();
    let jokes = task::spawn_blocking(move || {
        serde_json::from_slice::<Vec<WockaJoke>>(&jokes_data).unwrap()
    })
    .await
    .unwrap();
    // Filter out the following jokes:
    //
    // - Jokes with less than `MINIMUM_REDDIT_UPVOTES` score
    tokio_rayon::spawn(move || {
        jokes
            .into_iter()
            .map(|x| {
                let x: Joke = x.into();
                x
            })
            .collect::<Vec<_>>()
    })
    .await
}
