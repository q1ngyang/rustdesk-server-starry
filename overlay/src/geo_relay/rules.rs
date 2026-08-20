use crate::starry_config::{GeoConfig, GeoRuleConfig};

use super::GeoFacts;

const MAX_EXPRESSION_DEPTH: usize = 32;
const MAX_EXPRESSION_TERMS: usize = 256;

pub(super) struct RuleSet {
    rules: Vec<RelayRule>,
    requirements: DbRequirements,
}

pub(super) struct Selection {
    pub(super) relay: String,
    pub(super) rule_name: String,
    pub(super) rule_index: usize,
    pub(super) direction: &'static str,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DbRequirements {
    pub(super) country: bool,
    pub(super) city: bool,
    pub(super) asn: bool,
}

impl RuleSet {
    pub(super) fn empty() -> Self {
        Self {
            rules: Vec::new(),
            requirements: DbRequirements::default(),
        }
    }

    pub(super) fn compile(config: &GeoConfig) -> Result<Self, String> {
        if !config.enabled {
            return Ok(Self::empty());
        }

        let mut rules = Vec::with_capacity(config.rules.len());
        let mut requirements = DbRequirements::default();
        for rule in &config.rules {
            let compiled = RelayRule::compile(rule)?;
            requirements.merge(compiled.client_a.requirements());
            requirements.merge(compiled.client_b.requirements());
            rules.push(compiled);
        }
        Ok(Self {
            rules,
            requirements,
        })
    }

    pub(super) fn select(
        &self,
        facts_a: &GeoFacts,
        facts_b: &GeoFacts,
        online_relays: &[String],
    ) -> Option<Selection> {
        for (rule_index, rule) in self.rules.iter().enumerate() {
            let Some(direction) = rule.match_direction(facts_a, facts_b) else {
                continue;
            };
            if let Some(relay) = select_ordered(&rule.relays, online_relays) {
                return Some(Selection {
                    relay,
                    rule_name: rule.name.clone(),
                    rule_index,
                    direction,
                });
            }
        }
        None
    }

    pub(super) fn len(&self) -> usize {
        self.rules.len()
    }

    pub(super) fn requirements(&self) -> DbRequirements {
        self.requirements
    }
}

impl DbRequirements {
    fn merge(&mut self, other: Self) {
        self.country |= other.country;
        self.city |= other.city;
        self.asn |= other.asn;
    }
}

struct RelayRule {
    name: String,
    symmetric: bool,
    client_a: Expression,
    client_b: Expression,
    relays: Vec<String>,
}

impl RelayRule {
    fn compile(config: &GeoRuleConfig) -> Result<Self, String> {
        let client_a = ExpressionParser::parse(&config.matches.client_a).map_err(|err| {
            format!(
                "Geo rule '{}' has invalid client_a expression: {err}",
                config.name
            )
        })?;
        let client_b = ExpressionParser::parse(&config.matches.client_b).map_err(|err| {
            format!(
                "Geo rule '{}' has invalid client_b expression: {err}",
                config.name
            )
        })?;
        Ok(Self {
            name: config.name.clone(),
            symmetric: config.symmetric,
            client_a,
            client_b,
            relays: config.relays.clone(),
        })
    }

    fn match_direction(&self, facts_a: &GeoFacts, facts_b: &GeoFacts) -> Option<&'static str> {
        let direct = self.client_a.matches(facts_a) && self.client_b.matches(facts_b);
        if direct {
            Some("direct")
        } else if self.symmetric && self.client_a.matches(facts_b) && self.client_b.matches(facts_a)
        {
            Some("reverse")
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Expression {
    Any,
    Predicate(Predicate),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, Box<Expression>),
}

impl Expression {
    fn matches(&self, facts: &GeoFacts) -> bool {
        match self {
            Self::Any => true,
            Self::Predicate(predicate) => predicate.matches(facts),
            Self::And(left, right) => left.matches(facts) && right.matches(facts),
            Self::Or(left, right) => left.matches(facts) || right.matches(facts),
        }
    }

    fn requirements(&self) -> DbRequirements {
        match self {
            Self::Any => DbRequirements::default(),
            Self::Predicate(predicate) => predicate.requirements(),
            Self::And(left, right) | Self::Or(left, right) => {
                let mut requirements = left.requirements();
                requirements.merge(right.requirements());
                requirements
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Predicate {
    Continent(String),
    Country(String),
    Subdivision(String),
    City(String),
    Geoname(u32),
    Asn(u32),
    Isp(String),
}

impl Predicate {
    fn compile(raw: &str) -> Result<Expression, String> {
        let raw = raw.trim();
        if raw == "*" {
            return Ok(Expression::Any);
        }
        if is_country_code(raw) {
            return Ok(Expression::Predicate(Self::Country(
                raw.to_ascii_uppercase(),
            )));
        }

        let (field, value) = raw
            .split_once(':')
            .ok_or_else(|| format!("'{raw}' must be '*' or field:value"))?;
        let field = field.trim().to_ascii_lowercase();
        let value = decode_value(value)?;
        if value.is_empty() {
            return Err(format!("field '{field}' has an empty value"));
        }
        let predicate = match field.as_str() {
            "continent" => Self::Continent(value.to_ascii_uppercase()),
            "country" => {
                if !is_country_code(&value) {
                    return Err(format!("country '{value}' must be a two-letter code"));
                }
                Self::Country(value.to_ascii_uppercase())
            }
            "subdivision" | "region" => Self::Subdivision(value),
            "city" => Self::City(value),
            "geoname" | "city_id" => Self::Geoname(parse_nonzero_u32(&field, &value)?),
            "asn" => {
                let value = value
                    .strip_prefix("AS")
                    .or_else(|| value.strip_prefix("as"))
                    .unwrap_or(&value);
                Self::Asn(parse_nonzero_u32(&field, value)?)
            }
            "isp" | "asn_org" => Self::Isp(value),
            _ => {
                return Err(format!(
                    "unsupported field '{field}'; use continent, country, subdivision, city, geoname, asn, or isp"
                ))
            }
        };
        Ok(Expression::Predicate(predicate))
    }

    fn matches(&self, facts: &GeoFacts) -> bool {
        match self {
            Self::Continent(expected) => matches_optional(&facts.continent, expected),
            Self::Country(expected) => matches_optional(&facts.country, expected),
            Self::Subdivision(expected) => {
                matches_values(&facts.subdivision_codes, expected)
                    || matches_values(&facts.subdivision_names, expected)
            }
            Self::City(expected) => matches_values(&facts.city_names, expected),
            Self::Geoname(expected) => facts.city_geoname_id == Some(*expected),
            Self::Asn(expected) => facts.asn == Some(*expected),
            Self::Isp(expected) => matches_contains(&facts.asn_org, expected),
        }
    }

    fn requirements(&self) -> DbRequirements {
        match self {
            Self::Continent(_) | Self::Country(_) => DbRequirements {
                country: true,
                ..DbRequirements::default()
            },
            Self::Subdivision(_) | Self::City(_) | Self::Geoname(_) => DbRequirements {
                city: true,
                ..DbRequirements::default()
            },
            Self::Asn(_) | Self::Isp(_) => DbRequirements {
                asn: true,
                ..DbRequirements::default()
            },
        }
    }
}

struct ExpressionParser<'a> {
    source: &'a str,
    position: usize,
    depth: usize,
    terms: usize,
}

impl<'a> ExpressionParser<'a> {
    fn parse(source: &'a str) -> Result<Expression, String> {
        let mut parser = Self {
            source,
            position: 0,
            depth: 0,
            terms: 0,
        };
        let expression = parser.parse_or()?;
        parser.skip_whitespace();
        if parser.position != source.len() {
            return Err(format!("unexpected token at byte {}", parser.position));
        }
        Ok(expression)
    }

    fn parse_or(&mut self) -> Result<Expression, String> {
        let mut expression = self.parse_and()?;
        loop {
            self.skip_whitespace();
            if !self.consume('/') {
                break;
            }
            let right = self.parse_and()?;
            expression = Expression::Or(Box::new(expression), Box::new(right));
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expression, String> {
        let mut expression = self.parse_primary()?;
        loop {
            self.skip_whitespace();
            if !self.consume('+') {
                break;
            }
            let right = self.parse_primary()?;
            expression = Expression::And(Box::new(expression), Box::new(right));
        }
        Ok(expression)
    }

    fn parse_primary(&mut self) -> Result<Expression, String> {
        self.skip_whitespace();
        if self.consume('(') {
            if self.depth >= MAX_EXPRESSION_DEPTH {
                return Err(format!(
                    "expression nesting exceeds {MAX_EXPRESSION_DEPTH} levels"
                ));
            }
            self.depth += 1;
            let expression = self.parse_or()?;
            self.depth -= 1;
            self.skip_whitespace();
            if !self.consume(')') {
                return Err(format!("missing ')' at byte {}", self.position));
            }
            return Ok(expression);
        }
        let raw = self.read_predicate()?;
        self.terms += 1;
        if self.terms > MAX_EXPRESSION_TERMS {
            return Err(format!(
                "expression contains more than {MAX_EXPRESSION_TERMS} terms"
            ));
        }
        Predicate::compile(raw)
    }

    fn read_predicate(&mut self) -> Result<&'a str, String> {
        self.skip_whitespace();
        let start = self.position;
        let mut quote = None;
        let mut escaped = false;
        while let Some(ch) = self.peek() {
            if let Some(active_quote) = quote {
                self.advance(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            if ch == '\'' || ch == '"' {
                quote = Some(ch);
                self.advance(ch);
                continue;
            }
            if matches!(ch, '+' | '/' | '(' | ')') {
                break;
            }
            self.advance(ch);
        }
        if quote.is_some() {
            return Err(format!("unterminated quoted value at byte {start}"));
        }
        let value = self.source[start..self.position].trim();
        if value.is_empty() {
            Err(format!("missing expression term at byte {start}"))
        } else {
            Ok(value)
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if !ch.is_whitespace() {
                break;
            }
            self.advance(ch);
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance(expected);
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn advance(&mut self, ch: char) {
        self.position += ch.len_utf8();
    }
}

fn decode_value(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let Some(first) = raw.chars().next() else {
        return Ok(String::new());
    };
    if first != '\'' && first != '"' {
        return Ok(raw.to_owned());
    }
    if raw.len() < first.len_utf8() * 2 || !raw.ends_with(first) {
        return Err("quoted value must end with the same quote".to_owned());
    }
    let body_start = first.len_utf8();
    let body_end = raw.len() - first.len_utf8();
    let mut value = String::new();
    let mut escaped = false;
    for ch in raw[body_start..body_end].chars() {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            value.push(ch);
        }
    }
    if escaped {
        return Err("quoted value ends with an incomplete escape".to_owned());
    }
    Ok(value.trim().to_owned())
}

fn parse_nonzero_u32(field: &str, value: &str) -> Result<u32, String> {
    let value = value
        .parse::<u32>()
        .map_err(|_| format!("{field} '{value}' is not a valid integer"))?;
    if value == 0 {
        Err(format!("{field} must be greater than zero"))
    } else {
        Ok(value)
    }
}

fn select_ordered(configured_relays: &[String], online_relays: &[String]) -> Option<String> {
    for configured in configured_relays {
        if let Some(online) = online_relays
            .iter()
            .find(|online| online.eq_ignore_ascii_case(configured))
        {
            return Some(online.clone());
        }
    }
    None
}

fn matches_optional(actual: &Option<String>, expected: &str) -> bool {
    actual
        .as_ref()
        .map(|actual| actual.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn matches_values(actual: &[String], expected: &str) -> bool {
    actual
        .iter()
        .any(|actual| actual.eq_ignore_ascii_case(expected))
}

fn matches_contains(actual: &Option<String>, expected: &str) -> bool {
    let Some(actual) = actual.as_ref() else {
        return false;
    };
    actual
        .to_ascii_lowercase()
        .contains(&expected.to_ascii_lowercase())
}

fn is_country_code(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|ch| ch.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starry_config::{EndpointExpressions, GeoRuleConfig};

    fn facts(country: &str, city: &str, asn: u32, org: &str) -> GeoFacts {
        GeoFacts {
            country: Some(country.to_owned()),
            city_names: vec![city.to_owned()],
            asn: Some(asn),
            asn_org: Some(org.to_owned()),
            ..GeoFacts::default()
        }
    }

    fn config(expression: &str, relays: &[&str]) -> GeoConfig {
        GeoConfig {
            enabled: true,
            rules: vec![GeoRuleConfig {
                name: "test".to_owned(),
                symmetric: false,
                matches: EndpointExpressions {
                    client_a: expression.to_owned(),
                    client_b: "*".to_owned(),
                },
                relays: relays.iter().map(|relay| (*relay).to_owned()).collect(),
            }],
        }
    }

    #[test]
    fn slash_is_or_plus_is_and_and_parentheses_nest() {
        let expression = ExpressionParser::parse(
            "((city:上海+isp:China Telecom)/(city:首尔+isp:KT))+country:CN",
        )
        .unwrap();
        assert!(expression.matches(&facts("CN", "上海", 4134, "China Telecom")));
        assert!(!expression.matches(&facts("KR", "首尔", 4766, "KT")));
        assert!(!expression.matches(&facts("CN", "上海", 9808, "China Mobile")));
    }

    #[test]
    fn plus_has_higher_precedence_than_slash() {
        let expression = ExpressionParser::parse("city:上海+isp:Telecom/city:东京").unwrap();
        assert!(expression.matches(&facts("JP", "东京", 2516, "KDDI")));
        assert!(expression.matches(&facts("CN", "上海", 4134, "China Telecom")));
        assert!(!expression.matches(&facts("CN", "上海", 9808, "China Mobile")));
    }

    #[test]
    fn country_list_uses_or() {
        let expression = ExpressionParser::parse("CN/JP/KR/TW").unwrap();
        assert!(expression.matches(&facts("TW", "台北", 3462, "HiNet")));
        assert!(!expression.matches(&facts("US", "Seattle", 7922, "Comcast")));
    }

    #[test]
    fn relays_are_strictly_ordered_without_round_robin() {
        let rules = RuleSet::compile(&config("CN", &["relay-a", "relay-b"])).unwrap();
        let online = vec!["relay-b".to_owned(), "relay-a".to_owned()];
        for _ in 0..10 {
            let selected = rules
                .select(
                    &facts("CN", "上海", 4134, "China Telecom"),
                    &facts("US", "Seattle", 7922, "Comcast"),
                    &online,
                )
                .unwrap();
            assert_eq!(selected.relay, "relay-a");
        }
    }

    #[test]
    fn uses_next_relay_only_if_the_first_is_offline() {
        let rules = RuleSet::compile(&config("CN", &["relay-a", "relay-b"])).unwrap();
        let selected = rules
            .select(
                &facts("CN", "上海", 4134, "China Telecom"),
                &facts("US", "Seattle", 7922, "Comcast"),
                &["relay-b".to_owned()],
            )
            .unwrap();
        assert_eq!(selected.relay, "relay-b");
    }

    #[test]
    fn quoted_values_may_contain_operator_characters() {
        let expression = ExpressionParser::parse(r#"city:"A/B"+isp:'X+Y'"#).unwrap();
        assert!(expression.matches(&facts("US", "A/B", 64512, "Carrier X+Y")));
    }

    #[test]
    fn rejects_malformed_expressions() {
        for expression in ["CN/", "(CN/JP", "city:", "unknown:value", "USA"] {
            assert!(ExpressionParser::parse(expression).is_err(), "{expression}");
        }
    }

    #[test]
    fn bounds_expression_nesting_and_term_count() {
        let nested = format!(
            "{}CN{}",
            "(".repeat(MAX_EXPRESSION_DEPTH + 1),
            ")".repeat(MAX_EXPRESSION_DEPTH + 1)
        );
        assert!(ExpressionParser::parse(&nested)
            .unwrap_err()
            .contains("nesting exceeds"));

        let terms = std::iter::repeat("CN")
            .take(MAX_EXPRESSION_TERMS + 1)
            .collect::<Vec<_>>()
            .join("/");
        assert!(ExpressionParser::parse(&terms)
            .unwrap_err()
            .contains("more than"));
    }
}
