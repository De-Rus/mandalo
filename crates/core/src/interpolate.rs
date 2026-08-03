use crate::error::{CoreError, CoreResult};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn apply(template: &str, vars: &BTreeMap<String, String>) -> CoreResult<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| CoreError::Template(format!("unclosed {{{{ in: {template}")))?;
        let key = after[..end].trim();
        match vars.get(key) {
            Some(value) => out.push_str(value),
            None if key.starts_with('$') => out.push_str(&dynamic(key)?),
            None => {
                return Err(CoreError::UnresolvedVar(format!(
                    "unresolved variable: {key}"
                )))
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Postman's generated `{{$…}}` values. The list is mirrored by the script
/// sandbox so `pm.variables.replaceIn` produces the same shapes.
pub const DYNAMIC_VARIABLES: &[&str] = &[
    "$guid",
    "$randomUUID",
    "$timestamp",
    "$isoTimestamp",
    "$randomInt",
    "$randomBoolean",
    "$randomAlphaNumeric",
    "$randomWord",
    "$randomWords",
    "$randomFirstName",
    "$randomLastName",
    "$randomFullName",
    "$randomUserName",
    "$randomEmail",
    "$randomPassword",
    "$randomPhoneNumber",
    "$randomCity",
    "$randomCountry",
    "$randomStreetAddress",
    "$randomCompanyName",
    "$randomJobTitle",
    "$randomProductName",
    "$randomDomainName",
    "$randomUrl",
    "$randomIP",
    "$randomColor",
    "$randomHexColor",
    "$randomDatePast",
    "$randomDateFuture",
];

const FIRST: &[&str] = &["Ada", "Nova", "Iris", "Leo", "Mila", "Otto", "Sara", "Theo"];
const LAST: &[&str] = &[
    "Vega", "Rossi", "Ivanov", "Kaur", "Nakamura", "Silva", "Dubois", "Okafor",
];
const WORDS: &[&str] = &[
    "alpha", "bravo", "delta", "ember", "flint", "gamma", "harbor", "ivory",
];
const CITIES: &[&str] = &["Lisbon", "Osaka", "Bogota", "Tallinn", "Nairobi", "Quito"];
const COUNTRIES: &[&str] = &[
    "Portugal", "Japan", "Colombia", "Estonia", "Kenya", "Ecuador",
];
const COLORS: &[&str] = &["red", "blue", "green", "amber", "violet", "teal"];
const JOBS: &[&str] = &["Engineer", "Analyst", "Designer", "Architect", "Technician"];
const PRODUCTS: &[&str] = &["Keyboard", "Lamp", "Bicycle", "Notebook", "Kettle"];
const COMPANIES: &[&str] = &["Northwind", "Contoso", "Umbra", "Lumen", "Meridian"];

fn entropy() -> u64 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let mut value = 0u64;
    for byte in bytes.iter().take(8) {
        value = (value << 8) | u64::from(*byte);
    }
    value
}

fn pick(list: &[&str]) -> String {
    list[(entropy() % list.len() as u64) as usize].to_string()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn iso_timestamp(offset_secs: i64) -> String {
    let secs = (unix_seconds() as i64 + offset_secs).max(0) as u64;
    let days = secs / 86_400;
    let time = secs % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn dynamic(name: &str) -> CoreResult<String> {
    let value = match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => unix_seconds().to_string(),
        "$isoTimestamp" => iso_timestamp(0),
        "$randomInt" => (entropy() % 1001).to_string(),
        "$randomBoolean" => entropy().is_multiple_of(2).to_string(),
        "$randomAlphaNumeric" => {
            let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789";
            (alphabet[(entropy() % alphabet.len() as u64) as usize] as char).to_string()
        }
        "$randomWord" => pick(WORDS),
        "$randomWords" => format!("{} {} {}", pick(WORDS), pick(WORDS), pick(WORDS)),
        "$randomFirstName" => pick(FIRST),
        "$randomLastName" => pick(LAST),
        "$randomFullName" => format!("{} {}", pick(FIRST), pick(LAST)),
        "$randomUserName" => format!(
            "{}.{}",
            pick(FIRST).to_lowercase(),
            pick(LAST).to_lowercase()
        ),
        "$randomEmail" => format!(
            "{}.{}@example.com",
            pick(FIRST).to_lowercase(),
            pick(LAST).to_lowercase()
        ),
        "$randomPassword" => format!("{:012x}", entropy() & 0xffff_ffff_ffff),
        "$randomPhoneNumber" => format!("5{:02}-{}-{}", entropy() % 100, entropy() % 900, entropy() % 9000),
        "$randomCity" => pick(CITIES),
        "$randomCountry" => pick(COUNTRIES),
        "$randomStreetAddress" => format!("{} {} Street", entropy() % 400, pick(WORDS)),
        "$randomCompanyName" => pick(COMPANIES),
        "$randomJobTitle" => pick(JOBS),
        "$randomProductName" => pick(PRODUCTS),
        "$randomDomainName" => format!("{}.com", pick(COMPANIES).to_lowercase()),
        "$randomUrl" => format!("https://{}.com/{}", pick(COMPANIES).to_lowercase(), pick(WORDS)),
        "$randomIP" => format!(
            "{}.{}.{}.{}",
            entropy() % 256,
            entropy() % 256,
            entropy() % 256,
            entropy() % 256
        ),
        "$randomColor" => pick(COLORS),
        "$randomHexColor" => format!("#{:06x}", entropy() & 0xff_ffff),
        "$randomDatePast" => iso_timestamp(-((entropy() % 31_536_000) as i64)),
        "$randomDateFuture" => iso_timestamp((entropy() % 31_536_000) as i64),
        other => {
            return Err(CoreError::Unsupported(format!(
                "{{{{{other}}}}} is a Postman dynamic variable Mándalo cannot generate — supported ones are {}",
                DYNAMIC_VARIABLES.join(", ")
            )))
        }
    };
    Ok(value)
}

pub fn names(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return out;
        };
        let key = after[..end].trim();
        if !key.is_empty() && !key.starts_with('$') && !out.iter().any(|k: &String| k == key) {
            out.push(key.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn names_lists_referenced_keys_once_in_order() {
        assert_eq!(names("{{b}}/{{ a }}/{{b}}"), vec!["b", "a"]);
        assert!(names("no variables here").is_empty());
        assert!(names("{{unclosed").is_empty());
    }

    #[test]
    fn passthrough_without_variables() {
        assert_eq!(
            apply("https://api.example.com", &vars(&[])).unwrap(),
            "https://api.example.com"
        );
    }

    #[test]
    fn substitutes_single_variable() {
        let v = vars(&[("base", "https://api.example.com")]);
        assert_eq!(
            apply("{{base}}/users", &v).unwrap(),
            "https://api.example.com/users"
        );
    }

    #[test]
    fn substitutes_multiple_and_repeated() {
        let v = vars(&[("host", "x.dev"), ("v", "v2")]);
        assert_eq!(
            apply("https://{{host}}/{{v}}/{{v}}", &v).unwrap(),
            "https://x.dev/v2/v2"
        );
    }

    #[test]
    fn trims_whitespace_inside_braces() {
        let v = vars(&[("token", "abc")]);
        assert_eq!(apply("Bearer {{ token }}", &v).unwrap(), "Bearer abc");
    }

    #[test]
    fn unresolved_variable_fails_loud() {
        let err = apply("{{base}}/users", &vars(&[])).unwrap_err();
        assert_eq!(err.code(), "E_UNRESOLVED_VAR");
        let err = err.to_string();
        assert!(err.contains("unresolved variable: base"));
    }

    #[test]
    fn dynamic_variables_generate_postman_shapes() {
        let v = vars(&[]);
        let guid = apply("{{$guid}}", &v).unwrap();
        assert_eq!(guid.len(), 36);
        assert!(uuid::Uuid::parse_str(&guid).is_ok());
        assert_eq!(apply("{{$timestamp}}", &v).unwrap().len(), 10);
        assert!(apply("{{$isoTimestamp}}", &v).unwrap().ends_with('Z'));
        assert!(apply("{{$randomEmail}}", &v).unwrap().contains('@'));
        assert!(apply("{{$randomInt}}", &v)
            .unwrap()
            .parse::<u32>()
            .is_ok_and(|n| n <= 1000));
        assert_eq!(apply("{{$randomHexColor}}", &v).unwrap().len(), 7);
        let ip = apply("{{$randomIP}}", &v).unwrap();
        assert_eq!(ip.split('.').count(), 4);
    }

    #[test]
    fn every_advertised_dynamic_variable_resolves() {
        let v = vars(&[]);
        for name in DYNAMIC_VARIABLES {
            let out = apply(&format!("{{{{{name}}}}}"), &v).unwrap();
            assert!(!out.is_empty(), "{name} produced nothing");
        }
    }

    #[test]
    fn an_unknown_dynamic_variable_fails_loud_and_lists_the_supported_ones() {
        let err = apply("{{$randomBankAccount}}", &vars(&[])).unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        let err = err.to_string();
        assert!(err.contains("{{$randomBankAccount}}"));
        assert!(err.contains("$guid"));
    }

    #[test]
    fn a_declared_variable_wins_over_a_dynamic_one() {
        let v = vars(&[("$guid", "pinned")]);
        assert_eq!(apply("{{$guid}}", &v).unwrap(), "pinned");
    }

    #[test]
    fn names_ignores_dynamic_variables() {
        assert_eq!(names("{{base}}/{{$guid}}/{{$timestamp}}"), vec!["base"]);
    }

    #[test]
    fn unclosed_braces_fail_loud() {
        let err = apply("{{base/users", &vars(&[("base", "x")])).unwrap_err();
        assert_eq!(err.code(), "E_TEMPLATE");
        let err = err.to_string();
        assert!(err.contains("unclosed"));
    }
}
