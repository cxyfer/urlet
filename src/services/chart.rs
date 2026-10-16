//! Chart helpers: gap filling and server rendered SVG.
//!
//! Rendering the chart on the server keeps the browser bundle tiny and lets the
//! strict `script-src 'self'` policy stay in place.

use std::collections::HashMap;

use crate::db::clicks::DailyStat;
use crate::i18n;
use crate::routes::dto::{DailyPointDto, DimensionDto};
use crate::services::time;

/// Build a dense daily series covering the last `days` days (oldest first).
pub fn fill_days(series: &[DailyStat], days: i64) -> Vec<DailyPointDto> {
    let days = days.clamp(1, 365);
    let mut by_day: HashMap<&str, &DailyStat> = series
        .iter()
        .map(|stat| (stat.day.as_str(), stat))
        .collect();

    (0..days)
        .rev()
        .map(|offset| {
            let day = time::day_days_ago(offset);
            match by_day.remove(day.as_str()) {
                Some(stat) => DailyPointDto {
                    day,
                    clicks: stat.clicks,
                    unique_clicks: stat.unique_clicks,
                    bot_clicks: stat.bot_clicks,
                },
                None => DailyPointDto {
                    day,
                    clicks: 0,
                    unique_clicks: 0,
                    bot_clicks: 0,
                },
            }
        })
        .collect()
}

/// Render a stacked bar chart: human clicks in the accent colour, bots in grey.
///
/// The viewBox is normalised to `100 x 40` and stretched by CSS, so the markup
/// stays resolution independent and small. Only the HTML pages call this, so the
/// tooltips and the accessible name use the language of the current request.
pub fn svg_bars(points: &[DailyPointDto]) -> String {
    if points.is_empty() {
        return String::new();
    }
    let max = points
        .iter()
        .map(|point| point.clicks)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let slot = 100.0 / points.len() as f64;
    let width = (slot * 0.68).max(0.4);
    let chart_height = 38.0;

    let lang = i18n::current();
    let mut bars = String::new();
    for (index, point) in points.iter().enumerate() {
        let x = index as f64 * slot + (slot - width) / 2.0;
        let human = point.human_clicks();
        let bots = point.bot_clicks.max(0);
        let human_height = human as f64 / max * chart_height;
        let bot_height = bots as f64 / max * chart_height;

        if human_height > 0.0 {
            let title = i18n::format(
                lang,
                "chart.click_title",
                &[("clicks", human.to_string()), ("day", point.day.clone())],
            );
            bars.push_str(&format!(
                "<rect x=\"{x:.3}\" y=\"{:.3}\" width=\"{width:.3}\" height=\"{human_height:.3}\"><title>{title}</title></rect>",
                40.0 - human_height,
            ));
        }
        if bot_height > 0.0 {
            let title = i18n::format(
                lang,
                "chart.bot_click_title",
                &[("clicks", bots.to_string()), ("day", point.day.clone())],
            );
            bars.push_str(&format!(
                "<rect class=\"alt\" x=\"{x:.3}\" y=\"{:.3}\" width=\"{width:.3}\" height=\"{bot_height:.3}\"><title>{title}</title></rect>",
                40.0 - human_height - bot_height,
            ));
        }
    }

    format!(
        "<svg class=\"chart\" viewBox=\"0 0 100 40\" preserveAspectRatio=\"none\" role=\"img\" \
         aria-label=\"{}\">{bars}</svg>",
        lang.t("url_detail.chart.heading")
    )
}

/// Highest count in a breakdown, used to size the bars in the template.
pub fn max_count(items: &[DimensionDto]) -> i64 {
    items
        .iter()
        .map(|item| item.count)
        .max()
        .unwrap_or(0)
        .max(1)
}

/// Percentage width for a bar in a breakdown list.
pub fn bar_percent(count: i64, max: i64) -> i64 {
    if max <= 0 {
        return 0;
    }
    ((count as f64 / max as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(day: &str, clicks: i64, unique: i64, bots: i64) -> DailyStat {
        DailyStat {
            day: day.to_string(),
            clicks,
            unique_clicks: unique,
            bot_clicks: bots,
        }
    }

    #[test]
    fn fills_missing_days_with_zeroes() {
        let today = time::day_days_ago(0);
        let series = vec![stat(&today, 5, 3, 1)];
        let filled = fill_days(&series, 7);
        assert_eq!(filled.len(), 7);
        assert_eq!(filled.last().expect("today").clicks, 5);
        assert_eq!(filled.last().expect("today").unique_clicks, 3);
        assert_eq!(filled.first().expect("oldest").clicks, 0);
        assert!(filled.first().expect("oldest").day < filled.last().expect("today").day);
    }

    #[test]
    fn clamps_the_window() {
        assert_eq!(fill_days(&[], 0).len(), 1);
        assert_eq!(fill_days(&[], 10_000).len(), 365);
    }

    #[test]
    fn renders_svg_with_one_bar_per_day() {
        let points = fill_days(&[stat(&time::day_days_ago(0), 4, 4, 2)], 3);
        let svg = svg_bars(&points);
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert_eq!(
            svg.matches("<rect").count(),
            2,
            "one human bar and one bot bar"
        );
        assert!(svg.contains("aria-label"));
    }

    #[test]
    fn empty_charts_are_empty_strings() {
        assert!(svg_bars(&[]).is_empty());
    }

    #[test]
    fn computes_bar_percentages() {
        let items = vec![
            DimensionDto {
                label: "a".into(),
                count: 10,
            },
            DimensionDto {
                label: "b".into(),
                count: 5,
            },
        ];
        assert_eq!(max_count(&items), 10);
        assert_eq!(bar_percent(10, 10), 100);
        assert_eq!(bar_percent(5, 10), 50);
        assert_eq!(bar_percent(1, 0), 0);
    }
}
