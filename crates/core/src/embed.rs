//! Text embeddings for search by meaning: bge-small-en-v1.5 (int8 ONNX) behind a BERT WordPiece tokenizer.
//! CLS pooling, L2-normalised, so cosine similarity is a dot product.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ndarray::Array2;
use ort::session::Session;
use ort::value::TensorRef;

use crate::stt::ort_env;

pub const MODEL: &str = "model_quantized.onnx";
pub const VOCAB: &str = "vocab.txt";
/// Queries (not passages) get this in front, as the model was trained.
pub const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
pub const DIM: usize = 384;
const MAX_TOKENS: usize = 512;
const MAX_WORD_CHARS: usize = 100;

pub struct Tokenizer {
    vocab: HashMap<String, i64>,
    unk: i64,
    cls: i64,
    sep: i64,
}

impl Tokenizer {
    pub fn from_vocab(text: &str) -> Result<Self> {
        let vocab: HashMap<String, i64> = text.lines().enumerate().map(|(i, l)| (l.trim_end().to_string(), i as i64)).collect();
        let get = |t: &str| vocab.get(t).copied().with_context(|| format!("vocab has no {t}"));
        Ok(Self { unk: get("[UNK]")?, cls: get("[CLS]")?, sep: get("[SEP]")?, vocab })
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::from_vocab(&std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?)
    }

    /// Basic tokenisation: lowercase, strip accents, split on whitespace and punctuation, isolate CJK characters.
    pub fn basic(text: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut cur = String::new();
        for c in text.chars().flat_map(|c| c.to_lowercase()) {
            if (c.is_control() && !c.is_whitespace()) || c == '\u{fffd}' || is_mark(c) {
                continue;
            }
            if c.is_whitespace() {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else if is_punct(c) || is_cjk(c) {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                words.push(c.to_string());
            } else {
                cur.push(strip_accent(c));
            }
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        words
    }

    fn wordpiece(&self, word: &str, out: &mut Vec<i64>) {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > MAX_WORD_CHARS {
            out.push(self.unk);
            return;
        }
        let mark = out.len();
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found = None;
            while end > start {
                let piece: String = chars[start..end].iter().collect();
                let key = if start > 0 { format!("##{piece}") } else { piece };
                if let Some(&id) = self.vocab.get(&key) {
                    found = Some(id);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(id) => out.push(id),
                None => {
                    out.truncate(mark);
                    out.push(self.unk);
                    return;
                }
            }
            start = end;
        }
    }

    /// Token ids with [CLS] first and [SEP] last, at most 512 long.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![self.cls];
        for w in Self::basic(text) {
            self.wordpiece(&w, &mut ids);
            if ids.len() >= MAX_TOKENS - 1 {
                break;
            }
        }
        ids.truncate(MAX_TOKENS - 1);
        ids.push(self.sep);
        ids
    }
}

fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace() && !is_mark(c) && !c.is_control())
}

fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf | 0x20000..=0x2a6df | 0x2a700..=0x2ebef | 0xf900..=0xfaff | 0x2f800..=0x2fa1f)
}

/// Latin accents folded to the base letter (common precomposed range; combining marks are dropped above).
fn strip_accent(c: char) -> char {
    match c {
        '\u{e0}'..='\u{e5}' => 'a',
        '\u{e7}' => 'c',
        '\u{e8}'..='\u{eb}' => 'e',
        '\u{ec}'..='\u{ef}' => 'i',
        '\u{f1}' => 'n',
        '\u{f2}'..='\u{f6}' | '\u{f8}' => 'o',
        '\u{f9}'..='\u{fc}' => 'u',
        '\u{fd}' | '\u{ff}' => 'y',
        _ => c,
    }
}

pub struct TextEmbedder {
    session: Session,
    tok: Tokenizer,
}

impl TextEmbedder {
    /// `dir` holds the model and vocab (models/embed).
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let env = ort_env(dir)?;
        let session = Session::builder(&env)?
            .with_intra_threads(threads)
            .map_err(ort::Error::<()>::from)?
            .commit_from_file(dir.join(MODEL))
            .with_context(|| format!("loading {MODEL}"))?;
        Ok(Self { session, tok: Tokenizer::load(&dir.join(VOCAB))? })
    }

    /// One unit-length vector per text. Callers prefix queries with `QUERY_PREFIX`.
    pub fn embed(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let enc: Vec<Vec<i64>> = texts.iter().map(|t| self.tok.encode(t)).collect();
        let n = enc.iter().map(Vec::len).max().unwrap_or(2);
        let mut ids = Array2::<i64>::zeros((texts.len(), n));
        let mut mask = Array2::<i64>::zeros((texts.len(), n));
        let types = Array2::<i64>::zeros((texts.len(), n));
        for (r, e) in enc.iter().enumerate() {
            for (c, &id) in e.iter().enumerate() {
                ids[[r, c]] = id;
                mask[[r, c]] = 1;
            }
        }
        let out = self.session.run(ort::inputs![
            "input_ids" => TensorRef::from_array_view(&ids)?,
            "attention_mask" => TensorRef::from_array_view(&mask)?,
            "token_type_ids" => TensorRef::from_array_view(&types)?,
        ])?;
        let hidden = out["last_hidden_state"].try_extract_array::<f32>()?;
        let shape = hidden.shape().to_vec();
        anyhow::ensure!(shape.len() == 3 && shape[0] == texts.len(), "unexpected output shape {shape:?}");
        Ok((0..texts.len()).map(|r| crate::speakers::normalized((0..shape[2]).map(|d| hidden[[r, 0, d]]).collect())).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok() -> Tokenizer {
        let words = ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "hello", "world", ",", "un", "##aff", "##able", "cafe", "!", "\u{4e2d}"];
        Tokenizer::from_vocab(&words.join("\n")).unwrap()
    }

    #[test]
    fn basic_splits_punctuation_and_folds_accents() {
        assert_eq!(Tokenizer::basic("Hello, World!"), ["hello", ",", "world", "!"]);
        assert_eq!(Tokenizer::basic("  Caf\u{e9}\tnow "), ["cafe", "now"]);
        assert_eq!(Tokenizer::basic("a\u{4e2d}b"), ["a", "\u{4e2d}", "b"]);
    }

    #[test]
    fn wordpiece_greedy_longest_match() {
        let t = tok();
        // [CLS] hello , un ##aff ##able [SEP]
        assert_eq!(t.encode("Hello, unaffable"), [2, 4, 6, 7, 8, 9, 3]);
        // an unknown word becomes a single [UNK]
        assert_eq!(t.encode("zzz world"), [2, 1, 5, 3]);
        assert_eq!(t.encode("\u{4e2d}"), [2, 12, 3]);
    }

    #[test]
    fn truncates_to_512() {
        let t = tok();
        let ids = t.encode(&"hello ".repeat(2000));
        assert_eq!(ids.len(), 512);
        assert_eq!((ids[0], ids[511]), (2, 3));
    }
}
