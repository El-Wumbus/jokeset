use std::num::NonZeroU64;

use lazy_static::lazy_static;
use scraper::{Html, Selector};

use crate::{Error, WockaJoke};

lazy_static! {
    pub static ref CONTENT_SELECTOR: Selector = Selector::parse("div#content").unwrap();
    pub static ref TITLE_SELECTOR: Selector = Selector::parse("div#content h2").unwrap();
    pub static ref CONTENT_CONTENTS_SELCTOR: Selector = Selector::parse("td.contents").unwrap();
}

/// Extracts a singular joke from `wocka.com`.
/// If the joke cannot be found or is a "dirty" joke (requires sign-in)
/// it cannot be parsed and `None` is returned.
pub async fn extract_joke(id: NonZeroU64) -> Result<WockaJoke, Error> {
    let url = format!("http://www.wocka.com/{id}.html");
    let response = reqwest::get(&url).await?.text().await?;
    // println!("{response}");
    let document = Html::parse_document(&response);

    // Get the title of the joke, it's the first (and only level-two heading on the page).
    let title = document
        .select(&TITLE_SELECTOR)
        .next()
        .and_then(|x| x.text().next())
        .ok_or(Error::Unhandled("Malformed wocka.com HTML".into()))?
        .to_string();

    let joke_details_table = document
        .select(&CONTENT_CONTENTS_SELCTOR)
        .map(|x| x.text().collect::<Vec<_>>())
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let x = x.trim();
            if !x.is_empty() {
                Some(x.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // Get category position in table, we add one to it so we get the value of category.
    let category_position = joke_details_table
        .iter()
        .position(|x| x == "Category")
        .ok_or(Error::Unhandled("Malformed wocka.com HTML".into()))?
        + 1;

    let category = joke_details_table
        .get(category_position)
        .ok_or(Error::Unhandled("Malformed wocka.com HTML".into()))?.to_owned();

    // Select `div#content`, actually grab it's child nodes instead.
    // Filter out all but text nodes. Text nodes are trimmed. Then collect,
    // then join everything with line breaks as separators.
    let body = document
        .select(&CONTENT_SELECTOR)
        .map(|x| x.children())
        .next()
        .ok_or(Error::Unhandled("Malformed wocka.com HTML".into()))?
        .filter_map(|x| x.value().as_text().and_then(|x| Some(x.to_string())))
        .filter_map(|x| {
            let x = x.trim();
            if !x.is_empty() {
                Some(x.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        return Err(Error::Unhandled("Malformed wocka.com HTML".into()));
    }

    Ok(WockaJoke {
        title,
        body,
        id,
        category,
    })
}
