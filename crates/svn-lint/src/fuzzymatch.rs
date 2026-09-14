//! Port of the compiler's `fuzzymatch` (`1-parse/utils/fuzzymatch.js`):
//! the "Did you mean …?" suggestion attached to some errors.
//!
//! Candidates are ranked by n-gram cosine similarity (3-grams, then
//! 2-grams), the best 50 are re-scored by normalised Levenshtein
//! distance, and the top score wins if it is above 0.7.

use std::collections::HashMap;

const GRAM_SIZE_LOWER: usize = 2;
const GRAM_SIZE_UPPER: usize = 3;

/// The closest name in `names` to `name`, if it is close enough.
pub fn fuzzymatch<'a>(name: &str, names: &[&'a str]) -> Option<&'a str> {
    if names.is_empty() {
        return None;
    }
    let set = FuzzySet::new(names);
    let matches = set.get(name)?;
    let (score, value) = matches.first()?;
    (*score > 0.7).then_some(*value)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut current: Vec<usize> = vec![0; a.len() + 1];
    let mut prev = 0;
    for i in 0..=b.len() {
        for j in 0..=a.len() {
            let value = if i > 0 && j > 0 {
                if a[j - 1] == b[i - 1] {
                    prev
                } else {
                    current[j].min(current[j - 1]).min(prev) + 1
                }
            } else {
                i + j
            };
            prev = current[j];
            current[j] = value;
        }
    }
    current[a.len()]
}

fn distance(a: &str, b: &str) -> f64 {
    let d = levenshtein(a, b) as f64;
    1.0 - d / (a.chars().count().max(b.chars().count()) as f64)
}

/// `-` + lowercased value with everything outside `[\w, ]` removed + `-`.
fn simplified(value: &str) -> String {
    let mut s = String::from("-");
    for c in value.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' || c == ',' || c == ' ' {
            s.push(c);
        }
    }
    s.push('-');
    s
}

fn gram_counter(value: &str, gram_size: usize) -> HashMap<String, usize> {
    let simplified: Vec<char> = simplified(value).chars().collect();
    let mut result = HashMap::new();
    if simplified.len() >= gram_size {
        for i in 0..=simplified.len() - gram_size {
            let gram: String = simplified[i..i + gram_size].iter().collect();
            *result.entry(gram).or_insert(0) += 1;
        }
    }
    result
}

struct FuzzySet<'a> {
    exact: HashMap<String, &'a str>,
    /// gram size → (gram → [(item index, count)])
    match_dict: HashMap<usize, HashMap<String, Vec<(usize, usize)>>>,
    /// gram size → [(vector norm, normalised value)]
    items: HashMap<usize, Vec<(f64, String)>>,
}

impl<'a> FuzzySet<'a> {
    fn new(names: &[&'a str]) -> Self {
        let mut set = Self {
            exact: HashMap::new(),
            match_dict: HashMap::new(),
            items: HashMap::new(),
        };
        for name in names {
            set.add(name);
        }
        set
    }

    fn add(&mut self, value: &'a str) {
        let normalized = value.to_lowercase();
        if self.exact.contains_key(&normalized) {
            return;
        }
        for gram_size in GRAM_SIZE_LOWER..=GRAM_SIZE_UPPER {
            let items = self.items.entry(gram_size).or_default();
            let index = items.len();
            let counts = gram_counter(&normalized, gram_size);
            let mut sum_of_squares = 0.0;
            let dict = self.match_dict.entry(gram_size).or_default();
            for (gram, count) in counts {
                sum_of_squares += (count * count) as f64;
                dict.entry(gram).or_default().push((index, count));
            }
            items.push((sum_of_squares.sqrt(), normalized.clone()));
        }
        self.exact.insert(normalized, value);
    }

    fn get(&self, value: &str) -> Option<Vec<(f64, &'a str)>> {
        let normalized = value.to_lowercase();
        if let Some(exact) = self.exact.get(&normalized) {
            return Some(vec![(1.0, exact)]);
        }
        for gram_size in (GRAM_SIZE_LOWER..=GRAM_SIZE_UPPER).rev() {
            let results = self.get_for(&normalized, gram_size);
            if !results.is_empty() {
                return Some(results);
            }
        }
        None
    }

    fn get_for(&self, normalized: &str, gram_size: usize) -> Vec<(f64, &'a str)> {
        let counts = gram_counter(normalized, gram_size);
        let (Some(items), Some(dict)) =
            (self.items.get(&gram_size), self.match_dict.get(&gram_size))
        else {
            return Vec::new();
        };
        let mut matches: HashMap<usize, usize> = HashMap::new();
        let mut sum_of_squares = 0.0;
        for (gram, count) in &counts {
            sum_of_squares += (count * count) as f64;
            if let Some(entries) = dict.get(gram) {
                for (index, other) in entries {
                    *matches.entry(*index).or_insert(0) += count * other;
                }
            }
        }
        let norm = sum_of_squares.sqrt();
        let mut results: Vec<(f64, usize)> = matches
            .into_iter()
            .map(|(index, score)| (score as f64 / (norm * items[index].0), index))
            .collect();
        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(50);
        let mut rescored: Vec<(f64, usize)> = results
            .into_iter()
            .map(|(_, index)| (distance(&items[index].1, normalized), index))
            .collect();
        rescored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let Some(best) = rescored.first().map(|r| r.0) else {
            return Vec::new();
        };
        rescored
            .into_iter()
            .filter(|(score, _)| *score == best)
            .filter_map(|(score, index)| self.exact.get(&items[index].1).map(|v| (score, *v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fuzzymatch;

    #[test]
    fn suggests_close_names_only() {
        let names = ["clientWidth", "clientHeight", "value", "this"];
        assert_eq!(fuzzymatch("clientWidht", &names), Some("clientWidth"));
        // Too far by the compiler's own scoring (checked against it).
        assert_eq!(fuzzymatch("valeu", &names), None);
        assert_eq!(fuzzymatch("noAssignment", &names), None);
    }
}
