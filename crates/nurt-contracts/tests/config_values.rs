// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text forms of configuration values: parsing, canonical writing and serde.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::config::{
    AgeLimit, ByteLimit, DurationSetting, Percent, SizeLimit, SizeOrPercent, SizeSetting, StepLimit,
};
use serde_json::json;

#[test]
fn durations_parse_units_and_combinations() {
    let cases = [
        ("250ms", 250),
        ("90s", 90_000),
        ("30m", 1_800_000),
        ("1h30m", 5_400_000),
        ("1d", 86_400_000),
        ("1w", 604_800_000),
        ("0s", 0),
    ];
    for (text, millis) in cases {
        let parsed: DurationSetting = text.parse().unwrap();
        assert_eq!(parsed.as_millis(), millis, "{text}");
    }
}

#[test]
fn durations_reject_missing_or_unknown_units() {
    for text in ["", "30", "m", "30x", "30M", "1.5h", "-5s", "30 m", "30m5"] {
        assert!(
            text.parse::<DurationSetting>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn durations_are_written_with_the_largest_exact_unit() {
    let cases = [
        (1_500, "1500ms"),
        (90_000, "90s"),
        (120_000, "2m"),
        (5_400_000, "90m"),
        (86_400_000, "1d"),
        (604_800_000, "1w"),
        (0, "0s"),
    ];
    for (millis, text) in cases {
        assert_eq!(DurationSetting::from_millis(millis).to_string(), text);
    }
}

#[test]
fn sizes_parse_decimal_binary_fractions_and_bytes() {
    let cases = [
        ("0", 0),
        ("512", 512),
        ("512b", 512),
        ("1kb", 1_000),
        ("1kib", 1_024),
        ("256mib", 256 << 20),
        ("2gib", 2 << 30),
        ("1.5tb", 1_500_000_000_000),
        ("1.5kib", 1_536),
        ("1tib", 1 << 40),
    ];
    for (text, bytes) in cases {
        let parsed: SizeSetting = text.parse().unwrap();
        assert_eq!(parsed.bytes(), bytes, "{text}");
    }
}

#[test]
fn sizes_reject_malformed_text() {
    for text in [
        "", "gib", "1.gib", ".5gib", "1..5gib", "2 gib", "2GiB", "2gb2", "-1b",
    ] {
        assert!(
            text.parse::<SizeSetting>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn sizes_are_written_with_the_largest_exact_unit() {
    assert_eq!(SizeSetting::from_gib(2).to_string(), "2gib");
    assert_eq!(SizeSetting::from_mib(256).to_string(), "256mib");
    assert_eq!(SizeSetting::from_bytes(1_000_000).to_string(), "1mb");
    assert_eq!(SizeSetting::from_bytes(1_500).to_string(), "1500b");
    assert_eq!(SizeSetting::from_bytes(0).to_string(), "0b");
}

#[test]
fn percentages_keep_two_decimal_places() {
    assert_eq!("15%".parse::<Percent>().unwrap(), Percent::from_percent(15));
    assert_eq!("12.5%".parse::<Percent>().unwrap().hundredths(), 1250);
    assert_eq!("0.25%".parse::<Percent>().unwrap().to_string(), "0.25%");
    assert_eq!("12.5%".parse::<Percent>().unwrap().to_string(), "12.5%");
    for text in ["15", "%", "1.234%", "101%", "1.%", "x%"] {
        assert!(
            text.parse::<Percent>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn size_or_percent_distinguishes_by_suffix() {
    assert_eq!(
        "15%".parse::<SizeOrPercent>().unwrap(),
        SizeOrPercent::Percent(Percent::from_percent(15))
    );
    assert_eq!(
        "5gib".parse::<SizeOrPercent>().unwrap(),
        SizeOrPercent::Size(SizeSetting::from_gib(5))
    );
}

#[test]
fn step_limits_accept_range_and_unlimited() {
    assert_eq!("1".parse::<StepLimit>().unwrap(), StepLimit::Limited(1));
    assert_eq!(
        "100000".parse::<StepLimit>().unwrap(),
        StepLimit::Limited(100_000)
    );
    assert_eq!(
        "unlimited".parse::<StepLimit>().unwrap(),
        StepLimit::Unlimited
    );
    for text in ["0", "100001", "-1", "ten", "Unlimited", ""] {
        assert!(
            text.parse::<StepLimit>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn age_limits_accept_range_and_unlimited() {
    assert_eq!("10s".parse::<AgeLimit>().unwrap().to_string(), "10s");
    assert_eq!("365d".parse::<AgeLimit>().unwrap().to_string(), "365d");
    assert_eq!(
        "unlimited".parse::<AgeLimit>().unwrap(),
        AgeLimit::Unlimited
    );
    for text in ["9s", "366d", "30"] {
        assert!(
            text.parse::<AgeLimit>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn size_limits_accept_sizes_shares_and_unlimited() {
    assert_eq!(
        "20gib".parse::<SizeLimit>().unwrap(),
        SizeLimit::Bytes(SizeSetting::from_gib(20))
    );
    assert_eq!(
        "10%".parse::<SizeLimit>().unwrap(),
        SizeLimit::OfCapacity(Percent::from_percent(10))
    );
    assert_eq!(
        "50%free".parse::<SizeLimit>().unwrap(),
        SizeLimit::OfFree(Percent::from_percent(50))
    );
    assert_eq!(
        "100%free".parse::<SizeLimit>().unwrap().to_string(),
        "100%free"
    );
    assert_eq!(
        "unlimited".parse::<SizeLimit>().unwrap(),
        SizeLimit::Unlimited
    );
    for text in ["0%", "91%", "0%free", "101%free", "512kib", "free"] {
        assert!(
            text.parse::<SizeLimit>().is_err(),
            "{text:?} must be rejected"
        );
    }
}

#[test]
fn byte_limits_accept_sizes_and_unlimited() {
    assert_eq!(
        "unlimited".parse::<ByteLimit>().unwrap(),
        ByteLimit::Unlimited
    );
    assert_eq!(
        "256mib".parse::<ByteLimit>().unwrap(),
        ByteLimit::Bytes(SizeSetting::from_mib(256))
    );
}

#[test]
fn values_serialize_to_their_file_form() {
    assert_eq!(
        serde_json::to_value(StepLimit::Limited(30)).unwrap(),
        json!(30)
    );
    assert_eq!(
        serde_json::to_value(StepLimit::Unlimited).unwrap(),
        json!("unlimited")
    );
    assert_eq!(
        serde_json::to_value(DurationSetting::from_secs(120)).unwrap(),
        json!("2m")
    );
    assert_eq!(
        serde_json::to_value(SizeSetting::from_gib(2)).unwrap(),
        json!("2gib")
    );
    assert_eq!(
        serde_json::to_value(SizeLimit::OfFree(Percent::from_percent(50))).unwrap(),
        json!("50%free")
    );
}

#[test]
fn values_deserialize_from_numbers_and_strings() {
    assert_eq!(
        serde_json::from_value::<StepLimit>(json!(30)).unwrap(),
        StepLimit::Limited(30)
    );
    assert_eq!(
        serde_json::from_value::<StepLimit>(json!("30")).unwrap(),
        StepLimit::Limited(30)
    );
    assert_eq!(
        serde_json::from_value::<SizeSetting>(json!(4096)).unwrap(),
        SizeSetting::from_bytes(4096)
    );
    assert!(serde_json::from_value::<StepLimit>(json!(0)).is_err());
    assert!(serde_json::from_value::<StepLimit>(json!(-3)).is_err());
    assert!(serde_json::from_value::<DurationSetting>(json!(30)).is_err());
    assert!(serde_json::from_value::<DurationSetting>(json!(true)).is_err());
}

#[test]
fn values_round_trip_through_toml_and_cbor() {
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        steps: StepLimit,
        age: AgeLimit,
        size: SizeLimit,
        reserve: SizeOrPercent,
        wait: DurationSetting,
    }
    let sample = Sample {
        steps: StepLimit::Unlimited,
        age: AgeLimit::Limited(DurationSetting::from_secs(1800)),
        size: SizeLimit::OfCapacity(Percent::from_percent(10)),
        reserve: SizeOrPercent::Size(SizeSetting::from_gib(5)),
        wait: DurationSetting::from_millis(1500),
    };
    let text = toml::to_string(&sample).unwrap();
    assert_eq!(toml::from_str::<Sample>(&text).unwrap(), sample);
    let mut bytes = Vec::new();
    ciborium::into_writer(&sample, &mut bytes).unwrap();
    assert_eq!(
        ciborium::from_reader::<Sample, _>(bytes.as_slice()).unwrap(),
        sample
    );
}
