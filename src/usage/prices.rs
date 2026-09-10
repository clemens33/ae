//! Bundled API-equivalent list prices and fixed-point price arithmetic.

use crate::usage::Tokens;
use std::io::Read as _;
use std::path::{Path, PathBuf};

const CONFIG_MAX_BYTES: u64 = 1024 * 1024;

/// Token rates in micro-USD per one million tokens (`$0.20/M` = `200_000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

// Source: https://raw.githubusercontent.com/BerriAI/litellm/fbed17d567a62b14b8fc7d9ef13c5cd61a8d1ae0/model_prices_and_context_window.json
// Commit observed 2026-09-10. Base service tier/context and five-minute cache writes only.
const PRICES: &[(&str, Price)] = &[
    // Each row uses: input_cost_per_token, cache_creation_input_token_cost,
    // cache_read_input_token_cost, output_cost_per_token.
    // Source key: claude-fable-5-1
    (
        "claude-fable-5-1",
        Price::new(10_000_000, 12_500_000, 250_000, 50_000_000),
    ),
    // Source key: claude-fable-5
    (
        "claude-fable-5",
        Price::new(10_000_000, 12_500_000, 1_000_000, 50_000_000),
    ),
    // Source key: claude-opus-5
    (
        "claude-opus-5",
        Price::new(5_000_000, 6_250_000, 500_000, 25_000_000),
    ),
    // Source key: claude-opus-4-8
    (
        "claude-opus-4-8",
        Price::new(5_000_000, 6_250_000, 500_000, 25_000_000),
    ),
    // Source key: claude-sonnet-5
    (
        "claude-sonnet-5",
        Price::new(2_000_000, 2_500_000, 200_000, 10_000_000),
    ),
    // Source key: gpt-5.6-sol
    (
        "gpt-5.6-sol",
        Price::new(4_000_000, 5_000_000, 400_000, 20_000_000),
    ),
    // Source key: gpt-5.6-luna
    (
        "gpt-5.6-luna",
        Price::new(200_000, 250_000, 20_000, 1_200_000),
    ),
    // Source key: gpt-5.6-terra
    (
        "gpt-5.6-terra",
        Price::new(2_000_000, 2_500_000, 200_000, 12_000_000),
    ),
    // Source key: gpt-6-astra
    (
        "gpt-6-astra",
        Price::new(10_000_000, 12_500_000, 1_000_000, 50_000_000),
    ),
];

/// Bundled prices overlaid by validated `[prices]` aliases.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Book {
    overrides: Vec<(String, Price)>,
}

impl Book {
    /// Exact override wins, then the immutable bundled table.
    #[must_use]
    pub fn price(&self, model: &str) -> Option<Price> {
        self.overrides
            .iter()
            .find(|(known, _)| known == model)
            .map(|(_, price)| *price)
            .or_else(|| lookup(model))
    }
}

/// Why selected `[prices]` config could not be trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Unreadable(PathBuf),
    Malformed {
        file: PathBuf,
        line: usize,
        row: String,
    },
    DuplicateModel {
        model: String,
        first_alias: String,
        second_alias: String,
    },
}

/// A standalone price row or decimal was malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl ConfigError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Unreadable(_) => 1,
            Self::Malformed { .. } | Self::DuplicateModel { .. } => 2,
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(file) => {
                write!(out, "Error: config {} cannot be read.", file.display())
            }
            Self::Malformed { file, line, row } => write!(
                out,
                "Error: {}:{line}: malformed [prices] row: {row}",
                file.display()
            ),
            Self::DuplicateModel {
                model,
                first_alias,
                second_alias,
            } => write!(
                out,
                "Error: [prices] model '{model}' is named by both '{first_alias}' and '{second_alias}'."
            ),
        }
    }
}

/// Read layered `[prices]` rows with the shared INI row grammar.
///
/// # Errors
///
/// Returns the selected path and row when a file is unreadable, malformed, or
/// names one model through multiple aliases.
pub fn read(global: Option<&Path>, local: Option<&Path>) -> Result<Book, ConfigError> {
    let mut aliases: Vec<(String, String, Price)> = Vec::new();
    for file in [global, local].into_iter().flatten() {
        let text = read_config(file)?;
        overlay(file, &text, &mut aliases)?;
    }
    for (index, (first_alias, first_model, _)) in aliases.iter().enumerate() {
        if let Some((second_alias, _, _)) = aliases
            .iter()
            .skip(index + 1)
            .find(|(_, model, _)| model == first_model)
        {
            return Err(ConfigError::DuplicateModel {
                model: first_model.clone(),
                first_alias: first_alias.clone(),
                second_alias: second_alias.clone(),
            });
        }
    }
    Ok(Book {
        overrides: aliases
            .into_iter()
            .map(|(_, model, price)| (model, price))
            .collect(),
    })
}

fn read_config(file: &Path) -> Result<String, ConfigError> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: usage opens the same selected INI files as the shared config reader"
    )]
    let opened = std::fs::File::open(file).map_err(|_| ConfigError::Unreadable(file.to_owned()))?;
    let metadata = opened
        .metadata()
        .map_err(|_| ConfigError::Unreadable(file.to_owned()))?;
    if !metadata.file_type().is_file() || metadata.len() > CONFIG_MAX_BYTES {
        return Err(ConfigError::Unreadable(file.to_owned()));
    }
    let mut bytes = Vec::new();
    opened
        .take(CONFIG_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ConfigError::Unreadable(file.to_owned()))?;
    if u64::try_from(bytes.len()).unwrap_or(CONFIG_MAX_BYTES + 1) > CONFIG_MAX_BYTES {
        return Err(ConfigError::Unreadable(file.to_owned()));
    }
    String::from_utf8(bytes).map_err(|_| ConfigError::Unreadable(file.to_owned()))
}

fn overlay(
    file: &Path,
    text: &str,
    aliases: &mut Vec<(String, String, Price)>,
) -> Result<(), ConfigError> {
    let mut section = String::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(next) = crate::config::section_header(trimmed) {
            section = next;
            continue;
        }
        if section != "prices" {
            continue;
        }
        let Some((alias, value)) = crate::config::parse_entry(trimmed) else {
            return Err(ConfigError::Malformed {
                file: file.to_owned(),
                line: index + 1,
                row: raw.to_owned(),
            });
        };
        if seen.iter().any(|known| known == alias) {
            return Err(ConfigError::Malformed {
                file: file.to_owned(),
                line: index + 1,
                row: raw.to_owned(),
            });
        }
        seen.push(alias.to_owned());
        let (model, price) = parse_value(&value).map_err(|_| ConfigError::Malformed {
            file: file.to_owned(),
            line: index + 1,
            row: raw.to_owned(),
        })?;
        if let Some(at) = aliases.iter().position(|(known, _, _)| known == alias) {
            aliases[at] = (alias.to_owned(), model, price);
        } else {
            aliases.push((alias.to_owned(), model, price));
        }
    }
    Ok(())
}

impl Price {
    const fn new(input: u64, cache_write: u64, cache_read: u64, output: u64) -> Self {
        Self {
            input,
            cache_write,
            cache_read,
            output,
        }
    }
}

/// Exact model id, or that exact id with a `-YYYYMMDD` release suffix.
#[must_use]
pub fn lookup(model: &str) -> Option<Price> {
    PRICES
        .iter()
        .find(|(name, _)| *name == model)
        .or_else(|| {
            PRICES
                .iter()
                .filter(|(name, _)| dated_variant(model, name))
                .max_by_key(|(name, _)| name.len())
        })
        .map(|(_, price)| *price)
}

fn dated_variant(model: &str, base: &str) -> bool {
    model.strip_prefix(base).is_some_and(|suffix| {
        suffix.len() == 9
            && suffix.starts_with('-')
            && suffix[1..].bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Convert four USD-per-million-token decimal fields into fixed-point rates.
///
/// # Errors
///
/// Returns [`ParseError`] unless exactly four non-negative decimals with at
/// most six fractional digits are present.
pub fn parse_override(value: &str) -> Result<Price, ParseError> {
    let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
    let [input, write, read, output] = fields.as_slice() else {
        return Err(ParseError);
    };
    Ok(Price::new(
        decimal_rate(input)?,
        decimal_rate(write)?,
        decimal_rate(read)?,
        decimal_rate(output)?,
    ))
}

/// Parse one shared-INI-shaped `[prices]` alias row.
///
/// # Errors
///
/// Returns [`ParseError`] when the row or any rate is malformed.
pub fn parse_row(line: &str) -> Result<(String, Price), ParseError> {
    let (_, value) = crate::config::parse_entry(line.trim()).ok_or(ParseError)?;
    parse_value(&value)
}

fn parse_value(value: &str) -> Result<(String, Price), ParseError> {
    let mut fields = value.split(',').map(str::trim);
    let model = fields
        .next()
        .filter(|model| super::is_model_id(model))
        .ok_or(ParseError)?;
    let rates = fields.collect::<Vec<_>>();
    if rates.len() != 4 {
        return Err(ParseError);
    }
    let price = parse_override(&rates.join(","))?;
    Ok((model.to_owned(), price))
}

fn decimal_rate(text: &str) -> Result<u64, ParseError> {
    let (whole, fraction) = text.split_once('.').map_or((text, ""), |parts| parts);
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
    {
        return Err(ParseError);
    }
    let whole: u64 = whole.parse().map_err(|_| ParseError)?;
    let mut fractional = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| ParseError)?
    };
    for _ in fraction.len()..6 {
        fractional = fractional.checked_mul(10).ok_or(ParseError)?;
    }
    whole
        .checked_mul(1_000_000)
        .and_then(|n| n.checked_add(fractional))
        .ok_or(ParseError)
}

/// Price tokens in whole micro-USD, rounding half a micro-dollar upward.
#[must_use]
pub fn cost(tokens: Tokens, price: Price) -> Option<u64> {
    let total = u128::from(tokens.input)
        .checked_mul(u128::from(price.input))?
        .checked_add(u128::from(tokens.cache_write).checked_mul(u128::from(price.cache_write))?)?
        .checked_add(u128::from(tokens.cache_read).checked_mul(u128::from(price.cache_read))?)?
        .checked_add(u128::from(tokens.output).checked_mul(u128::from(price.output))?)?;
    u64::try_from(total.checked_add(500_000)? / 1_000_000).ok()
}
