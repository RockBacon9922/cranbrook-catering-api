use axum::{extract::Query, http::StatusCode, response::IntoResponse};
use chrono::{Datelike, Local, Month, NaiveDate};
use lambda_runtime::LambdaEvent;
use reqwest::Client;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct QueryParams {
    pub date: String,
    pub period: String,
}

#[derive(Serialize)]
pub struct MealResponse {
    pub date: String,
    pub period: String,
    pub meal: String,
}

pub async fn get_meal(Query(params): Query<QueryParams>) -> impl IntoResponse {
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
    let period = params.period.to_lowercase();
    let fetched = fetch_meal_for_date(date, &period).await;

    match fetched {
        Ok(Some(meal)) => axum::Json(MealResponse {
            date: format_date(date),
            period,
            meal,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            format!("Meal not found for {} {}", format_date(date), period),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("Failed to fetch menu data: {err}"),
        )
            .into_response(),
    }
}

pub fn build_client() -> anyhow::Result<Client> {
    let client = Client::builder()
        // Avoid macOS system proxy lookup that can panic in sandboxed contexts.
        .no_proxy()
        .user_agent("cranbrook-catering-api/0.1")
        .build()?;
    Ok(client)
}

pub async fn fetch_menu_links(client: &Client) -> anyhow::Result<Vec<(String, Option<NaiveDate>)>> {
    let resp = client
        .get("https://www.cranbrookschool.co.uk/school-information/cranbrook-catering/")
        .send()
        .await?
        .text()
        .await?;

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

pub fn parse_week_commencing(text: &str) -> Option<NaiveDate> {
    // Parse "Menu for w/c Monday 26th January 2026" format
    let re = regex::Regex::new(r"w/c\s+\w+\s+(\d+)(?:st|nd|rd|th)?\s+(\w+)\s+(\d{4})").ok()?;
    let caps = re.captures(text)?;

    let day = caps.get(1)?.as_str().parse::<u32>().ok()?;
    let month_str = caps.get(2)?.as_str();
    let year = caps.get(3)?.as_str().parse::<i32>().ok()?;

    let month: u32 = match month_str.to_lowercase().as_str() {
        "january" => Month::January,
        "february" => Month::February,
        "march" => Month::March,
        "april" => Month::April,
        "may" => Month::May,
        "june" => Month::June,
        "july" => Month::July,
        "august" => Month::August,
        "september" => Month::September,
        "october" => Month::October,
        "november" => Month::November,
        "december" => Month::December,
        _ => return None,
    } as u32;

    NaiveDate::from_ymd_opt(year, month, day)
}

pub fn parse_week_commencing_from_pdf_text(text: &str) -> Option<NaiveDate> {
    // Handles variants such as:
    // - "Week Commencing Monday 26th January 2026"
    // - "w/c Monday 26 January 2026"
    let re = regex::Regex::new(
        r"(?i)(?:week\s+commencing|w/c)\s+\w+\s+(\d+)(?:st|nd|rd|th)?\s+(\w+)\s+(\d{4})",
    )
    .ok()?;
    let caps = re.captures(text)?;

    let day = caps.get(1)?.as_str().parse::<u32>().ok()?;
    let month_str = caps.get(2)?.as_str();
    let year = caps.get(3)?.as_str().parse::<i32>().ok()?;

    let month: u32 = match month_str.to_lowercase().as_str() {
        "january" => Month::January,
        "february" => Month::February,
        "march" => Month::March,
        "april" => Month::April,
        "may" => Month::May,
        "june" => Month::June,
        "july" => Month::July,
        "august" => Month::August,
        "september" => Month::September,
        "october" => Month::October,
        "november" => Month::November,
        "december" => Month::December,
        _ => return None,
    } as u32;

    NaiveDate::from_ymd_opt(year, month, day)
}

fn choose_inferred_week_start(
    week_starts: &[NaiveDate],
    requested_date: NaiveDate,
    today: NaiveDate,
) -> Option<NaiveDate> {
    if week_starts.is_empty() {
        return None;
    }

    // Prefer an exact containing week first.
    for week_start in week_starts {
        let week_end = *week_start + chrono::Duration::days(6);
        if requested_date >= *week_start && requested_date <= week_end {
            return Some(*week_start);
        }
    }

    // Infer target menu week from the menu week that best represents "today".
    let today_week = week_starts
        .iter()
        .min_by_key(|candidate| (today - **candidate).num_days().abs())
        .copied()?;

    let delta_weeks = (requested_date - today).num_days().div_euclid(7);
    let inferred_target = today_week + chrono::Duration::days(delta_weeks * 7);

    week_starts
        .iter()
        .min_by_key(|candidate| (inferred_target - **candidate).num_days().abs())
        .copied()
}

pub fn parse_date_param(input: &str) -> Option<NaiveDate> {
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

pub async fn download_and_extract_text(client: &Client, url: &str) -> anyhow::Result<String> {
    let bytes = client.get(url).send().await?.bytes().await?;
    let text = pdf_extract::extract_text_from_mem(&bytes)?;
    Ok(text)
}

pub fn is_junk_line(trimmed: &str, lower: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.chars().all(|c| !c.is_alphanumeric()) {
        return true;
    }
    if trimmed == "\"" {
        return true;
    }
    if lower.contains("cranbrook") || lower.contains("menu") || lower.contains("week commencing") {
        return true;
    }
    false
}

pub fn split_blocks(lines: &[String], expected_blocks: usize) -> Vec<Vec<String>> {
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

pub fn fill_first_line_per_day(
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

pub fn parse_weekly_menu(text: &str, week_start: NaiveDate) -> HashMap<String, String> {
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

pub async fn build_index() -> anyhow::Result<HashMap<String, String>> {
    let client = build_client()?;
    let links = fetch_menu_links(&client).await?;

    let mut index = HashMap::new();
    for (link, week_start_opt) in links {
        println!("Processing {link}");
        let text = download_and_extract_text(&client, &link).await?;

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

pub async fn fetch_meal_for_date(date: NaiveDate, period: &str) -> anyhow::Result<Option<String>> {
    let client = build_client()?;
    let links = fetch_menu_links(&client).await?;

    let mut menus: Vec<(String, NaiveDate, Option<String>)> = Vec::new();

    for (link, week_start_opt) in links {
        if let Some(week_start) = week_start_opt {
            menus.push((link, week_start, None));
            continue;
        }

        // If week start is not available in anchor text, inspect the PDF content.
        let text = match download_and_extract_text(&client, &link).await {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(week_start) = parse_week_commencing_from_pdf_text(&text) {
            menus.push((link, week_start, Some(text)));
        }
    }

    let week_starts: Vec<NaiveDate> = menus.iter().map(|(_, week_start, _)| *week_start).collect();
    let today = Local::now().date_naive();
    let target_week_start = match choose_inferred_week_start(&week_starts, date, today) {
        Some(value) => value,
        None => return Ok(None),
    };

    let mut matched: Option<(String, Option<String>)> = None;
    for (link, week_start, cached_text) in menus {
        if week_start == target_week_start {
            matched = Some((link, cached_text));
            break;
        }
    }
    let (link, cached_text) = match matched {
        Some(value) => value,
        None => return Ok(None),
    };

    let text = match cached_text {
        Some(value) => value,
        None => download_and_extract_text(&client, &link).await?,
    };
    let week_menus = parse_weekly_menu(&text, target_week_start);
    let period_key = period.to_lowercase();
    let key = format!("{}-{}", format_date(date), period_key);

    if let Some(meal) = week_menus.get(&key) {
        return Ok(Some(meal.clone()));
    }

    // If we inferred a nearby week, map by weekday within that inferred week.
    let weekday_offset = date.weekday().num_days_from_monday() as i64;
    let mapped_date = target_week_start + chrono::Duration::days(weekday_offset);
    let mapped_key = format!("{}-{}", format_date(mapped_date), period_key);
    Ok(week_menus.get(&mapped_key).cloned())
}

pub fn parse_payload<T>(event: &LambdaEvent<Value>, key: &str) -> Option<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    event
        .payload
        .get(key)
        .or_else(|| {
            event
                .payload
                .get("queryStringParameters")
                .and_then(|qs| qs.get(key))
        })
        .or_else(|| {
            event
                .payload
                .get("pathParameters")
                .and_then(|pp| pp.get(key))
        })
        .and_then(|v| {
            // If it's a string, try to deserialize it directly to the target type
            if let Some(s) = v.as_str() {
                // First try to deserialize the string directly to T
                if let Ok(result) = serde_json::from_str::<T>(s) {
                    return Some(result);
                }
                // If that fails, try wrapping it in quotes and deserializing
                let quoted = format!("\"{}\"", s);
                if let Ok(result) = serde_json::from_str::<T>(&quoted) {
                    return Some(result);
                }
            }
            serde_json::from_value(v.clone()).ok()
        })
}
