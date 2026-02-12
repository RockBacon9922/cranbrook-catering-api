use cargo_lambda_macro::lambda_function;
use lambda_runtime::{Error, LambdaEvent};
use serde_json::{Value, json};

use cranbrook_catering_api::{fetch_meal_for_date, parse_date_param, parse_payload};

fn build_response(status: u16, body: Value) -> Value {
    json!({
        "statusCode": status,
        "headers": { "content-type": "application/json" },
        "body": body.to_string(),
    })
}

#[lambda_function]
async fn func(event: LambdaEvent<Value>) -> Result<Value, Error> {
    // Run blocking network/PDF work on a dedicated blocking thread to avoid
    // dropping a nested Tokio runtime inside async context.

    let date_raw = parse_payload::<String>(&event, "date");
    let period_raw = parse_payload::<String>(&event, "period");

    let date_raw = match date_raw {
        Some(value) => value,
        None => chrono::Local::now().format("%Y-%m-%d").to_string(),
    };
    let period = match period_raw {
        Some(value) => value.to_lowercase(),
        None => {
            return Ok(build_response(
                400,
                json!({ "error": "Missing required 'period' parameter." }),
            ));
        }
    };

    let date = match parse_date_param(&date_raw) {
        Some(value) => value,
        None => {
            return Ok(build_response(
                400,
                json!({ "error": "Invalid date format. Use YYYY-MM-DD or YYYY/MM/DD." }),
            ));
        }
    };

    let fetched = fetch_meal_for_date(date, &period).await;

    match fetched {
        Ok(Some(meal)) => Ok(build_response(
            200,
            json!({
                "date": date_raw,
                "period": period,
                "meal": meal,
            }),
        )),
        Ok(None) => Ok(build_response(
            404,
            json!({ "error": "Meal not found for requested date/period." }),
        )),
        Err(err) => Ok(build_response(
            500,
            json!({ "error": format!("Failed to fetch menu data: {err}") }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    const DATE: NaiveDate = NaiveDate::from_ymd_opt(2026, 2, 12).unwrap();

    #[tokio::test]
    async fn test_lunch() {
        let period = "lunch";

        let fetched = fetch_meal_for_date(DATE, period).await.unwrap().unwrap();

        dbg!(&fetched);
    }
}
