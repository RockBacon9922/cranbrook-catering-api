use axum::{Router, extract::Query, http::StatusCode, response::IntoResponse, routing::get};
use chrono::{Datelike, NaiveDate};
use reqwest::Url;
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};

type MenuIndex = Arc<Mutex<HashMap<String, String>>>;

#[derive(Deserialize)]
struct QueryParams {
    date: String,
    period: String,
}

#[derive(Serialize)]
struct MealResponse {
    date: String,
    period: String,
    meal: String,
}

async fn get_meal(
    Query(params): Query<QueryParams>,
    menu_index: axum::extract::Extension<MenuIndex>,
) -> impl IntoResponse {
    let date = match parse_date_param(&params.date) {
        Some(date) => date,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid date format. Use YYYY-MM-DD or YYYY/MM/DD.",
            )
                .into_response();
        }
    };
    let period = &params.period.to_lowercase();
    let key = format!("{}-{period}", format_date(date));

    let index = menu_index.lock().unwrap();
    if let Some(meal) = index.get(&key) {
        axum::Json(MealResponse {
            date: format_date(date),
            period: period.clone(),
            meal: meal.clone(),
        })
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("Meal not found for {} {}", format_date(date), period),
        )
            .into_response()
    }
}

fn build_client() -> anyhow::Result<Client> {
    let client = Client::builder()
        // Avoid macOS system proxy lookup that can panic in sandboxed contexts.
        .no_proxy()
        .user_agent("cranbrook-catering-api/0.1")
        .build()?;
    Ok(client)
}

fn fetch_menu_links(client: &Client) -> anyhow::Result<Vec<(String, Option<NaiveDate>)>> {
    let resp = client
        .get("https://www.cranbrookschool.co.uk/school-information/cranbrook-catering/")
        .send()?
        .text()?;

    let doc = Html::parse_document(&resp);
    let selector = Selector::parse("a").unwrap();
    let base = Url::parse("https://www.cranbrookschool.co.uk/")?;

    let mut links = Vec::new();
    for element in doc.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            let href_lower = href.to_lowercase();
            if href_lower.contains(".pdf") && href_lower.contains("menu") {
                let link = match base.join(href) {
                    Ok(link) => link,
                    Err(_) => continue,
                };
                let link_text = element.text().collect::<String>();
                let week_date = parse_week_commencing(&link_text);
                links.push((link.to_string(), week_date));
            }
        }
    }

    Ok(links)
}

fn parse_week_commencing(text: &str) -> Option<NaiveDate> {
    // Parse "Menu for w/c Monday 26th January 2026" format
    let re = regex::Regex::new(r"w/c\s+\w+\s+(\d+)(?:st|nd|rd|th)?\s+(\w+)\s+(\d{4})").ok()?;
    let caps = re.captures(text)?;

    let day = caps.get(1)?.as_str().parse::<u32>().ok()?;
    let month_str = caps.get(2)?.as_str();
    let year = caps.get(3)?.as_str().parse::<i32>().ok()?;

    let month = match month_str.to_lowercase().as_str() {
        "january" => 1,
        "february" => 2,
        "march" => 3,
        "april" => 4,
        "may" => 5,
        "june" => 6,
        "july" => 7,
        "august" => 8,
        "september" => 9,
        "october" => 10,
        "november" => 11,
        "december" => 12,
        _ => return None,
    };

    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_date_param(input: &str) -> Option<NaiveDate> {
    let parts: Vec<_> = input
        .split(|c| c == '-' || c == '/')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let year = parts[0].parse::<i32>().ok()?;
    let month = parts[1].parse::<u32>().ok()?;
    let day = parts[2].parse::<u32>().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

fn format_date(date: NaiveDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

fn download_and_extract_text(client: &Client, url: &str) -> anyhow::Result<String> {
    let bytes = client.get(url).send()?.bytes()?;
    let text = pdf_extract::extract_text_from_mem(&bytes)?;
    Ok(text)
}

fn is_junk_line(trimmed: &str, lower: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.chars().all(|c| !c.is_alphanumeric()) {
        return true;
    }
    if trimmed == "\"" {
        return true;
    }
    false
}

fn split_blocks(lines: &[String], expected_blocks: usize) -> Vec<Vec<String>> {
    let mut blocks: Vec<Vec<String>> = Vec::new();
    for raw in lines {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();
        if is_junk_line(trimmed, &lower) {
            continue;
        }
        let starts_new =
            raw.starts_with(' ') && !blocks.is_empty() && blocks.len() < expected_blocks;
        if starts_new {
            blocks.push(Vec::new());
        }
        if blocks.is_empty() {
            blocks.push(Vec::new());
        }
        blocks.last_mut().unwrap().push(trimmed.to_string());
    }
    blocks
}

fn fill_first_line_per_day(
    lines: &[String],
    week_start: NaiveDate,
    days: usize,
    period: &str,
    out: &mut HashMap<String, String>,
) {
    let mut found = vec![false; days];
    for raw in lines {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();
        if is_junk_line(trimmed, &lower) {
            continue;
        }
        for day in 0..days {
            if !found[day] {
                let date = week_start + chrono::Duration::days(day as i64);
                let key = format!("{}-{period}", format_date(date));
                out.insert(key, trimmed.to_string());
                found[day] = true;
                break;
            }
        }
    }
}

fn parse_weekly_menu(text: &str, week_start: NaiveDate) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let lines: Vec<_> = text.lines().collect();

    // The PDF has a table structure where days are columns
    // We need to track which section (breakfast/brunch/lunch/dinner) we're in
    // and parse the first non-empty item for each day in that section

    let mut in_breakfast = false;
    let mut in_brunch_sat = false;
    let mut in_brunch_sun = false;
    let mut in_lunch = false;
    let mut in_dinner = false;

    let mut breakfast_found = [false; 5]; // Mon-Fri
    let mut brunch_sat_found = false;
    let mut brunch_sun_found = false;
    let mut lunch_lines: Vec<String> = Vec::new();
    let mut dinner_lines: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();

        // Detect section headers - look for lines with multiple instances of the period name
        let breakfast_count = lower.matches("breakfast").count();
        let brunch_count = lower.matches("brunch").count();
        let lunch_count = lower.matches("lunch").count();
        let dinner_count = lower.matches("dinner").count();

        if breakfast_count >= 3 {
            in_breakfast = true;
            in_brunch_sat = false;
            in_brunch_sun = false;
            in_lunch = false;
            in_dinner = false;
            continue;
        }
        if brunch_count >= 1 && !in_brunch_sat {
            in_breakfast = false;
            in_brunch_sat = true;
            in_brunch_sun = false;
            in_lunch = false;
            in_dinner = false;
            continue;
        }
        if brunch_count >= 1 && in_brunch_sat {
            in_brunch_sat = false;
            in_brunch_sun = true;
            continue;
        }
        if lunch_count >= 3 {
            in_breakfast = false;
            in_brunch_sat = false;
            in_brunch_sun = false;
            in_lunch = true;
            in_dinner = false;
            continue;
        }
        if dinner_count >= 3 {
            in_breakfast = false;
            in_brunch_sat = false;
            in_brunch_sun = false;
            in_lunch = false;
            in_dinner = true;
            continue;
        }

        if in_lunch {
            lunch_lines.push(line.to_string());
        } else if in_dinner {
            dinner_lines.push(line.to_string());
        }

        // Skip empty lines and lines with common filler text
        if is_junk_line(trimmed, &lower) {
            continue;
        }

        // Extract first meal for each day in current section
        if in_breakfast {
            for day in 0..5 {
                if !breakfast_found[day] && !trimmed.is_empty() {
                    let date = week_start + chrono::Duration::days(day as i64);
                    let key = format!("{}-breakfast", format_date(date));
                    out.insert(key, trimmed.to_string());
                    breakfast_found[day] = true;
                    break;
                }
            }
        } else if in_brunch_sat && !brunch_sat_found {
            let date = week_start + chrono::Duration::days(5); // Saturday
            let key = format!("{}-brunch", format_date(date));
            out.insert(key, "Brunch buffet available".to_string());
            brunch_sat_found = true;
        } else if in_brunch_sun && !brunch_sun_found {
            let date = week_start + chrono::Duration::days(6); // Sunday
            let key = format!("{}-brunch", format_date(date));
            out.insert(key, "Brunch buffet available".to_string());
            brunch_sun_found = true;
        }
    }

    let lunch_blocks = split_blocks(&lunch_lines, 5);
    if lunch_blocks.len() == 5 {
        for day in 0..5 {
            if let Some(block) = lunch_blocks.get(day) {
                let date = week_start + chrono::Duration::days(day as i64);
                let key = format!("{}-lunch", format_date(date));
                out.insert(key, block.join("\n"));
            }
        }
    } else {
        fill_first_line_per_day(&lunch_lines, week_start, 5, "lunch", &mut out);
    }

    let dinner_blocks = split_blocks(&dinner_lines, 7);
    if dinner_blocks.len() == 7 {
        for day in 0..7 {
            if let Some(block) = dinner_blocks.get(day) {
                let date = week_start + chrono::Duration::days(day as i64);
                let key = format!("{}-dinner", format_date(date));
                out.insert(key, block.join("\n"));
            }
        }
    } else {
        fill_first_line_per_day(&dinner_lines, week_start, 7, "dinner", &mut out);
    }

    out
}

fn build_index() -> anyhow::Result<HashMap<String, String>> {
    let client = build_client()?;
    let links = fetch_menu_links(&client)?;

    let mut index = HashMap::new();
    for (link, week_start_opt) in links {
        println!("Processing {link}");
        let text = download_and_extract_text(&client, &link)?;

        if let Some(week_start) = week_start_opt {
            println!("Week starting: {}", week_start);
            let week_menus = parse_weekly_menu(&text, week_start);

            for (k, v) in week_menus {
                println!("Storing key: {} -> {}", k, v);
                index.insert(k, v);
            }
        } else {
            println!("Skipping - could not parse week start date");
        }
    }

    println!("\nTotal entries in index: {}", index.len());
    println!(
        "Sample keys: {:?}",
        index.keys().take(5).collect::<Vec<_>>()
    );

    Ok(index)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Run blocking network/PDF work on a dedicated blocking thread to avoid
    // dropping a nested Tokio runtime inside async context.
    let index = tokio::task::spawn_blocking(build_index).await??;
    let shared_index = Arc::new(Mutex::new(index));

    let app = Router::new()
        .route("/meal", get(get_meal))
        .layer(axum::extract::Extension(shared_index))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await?;

    Ok(())
}
