//! core/frames.js: the frames posters and films of the day come in, shared
//! by the page, the desktop app and the xenonet film command.

use serde_json::{json, Value};
use xenonet_base::text::js_number;
use xenonet_base::units::js_round;

/// A poster's sizes, in pixels: a story (9:16), a portrait (4:5), a link
/// card and a wide frame (16:9).
pub fn poster_sizes() -> Value {
    json!([
        { "id": "story", "width": 1080, "height": 1920, "label": "Story", "ratio": "9:16" },
        { "id": "portrait", "width": 1080, "height": 1350, "label": "Portrait", "ratio": "4:5" },
        { "id": "card", "width": 1200, "height": 630, "label": "Link card", "ratio": "1.91:1" },
        { "id": "wide", "width": 1920, "height": 1080, "label": "Wide", "ratio": "16:9" },
    ])
}

pub const DEFAULT_SIZE: &str = "portrait";

/// Which of the poster sizes an id is, by its place: the default's for an
/// id that isn't one.
pub fn poster_size(id: Option<&str>) -> usize {
    let sizes = poster_sizes();
    let at = |want: &str| sizes.as_array().unwrap().iter().position(|s| s["id"] == want);
    id.and_then(at).or_else(|| at(DEFAULT_SIZE)).unwrap()
}

/// A film's frames: a story, the page's, and a wide one for --size.
pub fn film_frames() -> Value {
    json!({ "story": { "width": 1080, "height": 1920 }, "wide": { "width": 1920, "height": 1080 } })
}

pub const DEFAULT_FRAME: &str = "story";
pub const FILM_SECONDS: (f64, f64, f64) = (15.0, 30.0, 20.0);
pub const FILM_CARDS_MS: f64 = 8000.0;

/// How long a film is, cards included: a whole number of seconds from 15
/// to 30, 20 when it isn't said (or rounds to nothing).
pub fn film_seconds(total: f64) -> f64 {
    let (min, max, default) = FILM_SECONDS;
    let r = js_round(total);
    let r = if r == 0.0 || r.is_nan() { default } else { r };
    min.max(max.min(r))
}

pub fn film_play_seconds(total: f64) -> f64 {
    film_seconds(total) - FILM_CARDS_MS / 1000.0
}

// ToInt32, as JS's bitwise operators read a number.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc().rem_euclid(4294967296.0);
    (if m >= 2147483648.0 { m - 4294967296.0 } else { m }) as i32
}

/// The largest size in a frame's shape that fits an area, in even numbers
/// of pixels, which the video's pixel format needs.
pub fn fit_frame(frame: &Value, area: &Value) -> Value {
    let get = |v: &Value, k: &str| js_number(v.get(k));
    let (fw, fh) = (get(frame, "width"), get(frame, "height"));
    let k = [1.0, get(area, "width") / fw, get(area, "height") / fh].into_iter().fold(f64::INFINITY, |a, b| if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) });
    let even = |n: f64| 2.0f64.max(f64::from(to_int32(js_round(n)) & !1));
    json!({ "width": xenonet_base::json::number(even(fw * k)), "height": xenonet_base::json::number(even(fh * k)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn films_and_frames() {
        assert_eq!(film_seconds(f64::NAN), 20.0);
        assert_eq!(film_seconds(5.0), 15.0);
        assert_eq!(film_seconds(90.0), 30.0);
        assert_eq!(film_seconds(0.2), 20.0);
        assert_eq!(film_play_seconds(15.0), 7.0);
        assert_eq!(poster_size(Some("nope")), 1);
        assert_eq!(fit_frame(&film_frames()["wide"], &json!({ "width": 1440, "height": 900 })), json!({ "width": 1440, "height": 810 }));
        assert_eq!(fit_frame(&json!({}), &json!({ "width": 10, "height": 10 })), json!({ "width": 2, "height": 2 }));
    }
}
