use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub const RESTRICTED_KEYWORDS: &[&str] = &["boundry", "concept", "rule"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VocabularyKind {
    Boundary,
    Concept,
}

#[derive(Clone)]
pub struct VocabularyEntry {
    pub kind: VocabularyKind,
    pub uri: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Default)]
pub struct Vocabulary {
    pub entries: BTreeMap<String, Vec<VocabularyEntry>>,
}

impl Vocabulary {
    pub fn from_sources(sources: &HashMap<String, String>) -> Self {
        let mut vocabulary = Self::default();
        for (uri, text) in sources {
            if !uri.ends_with(".spec") && !uri.ends_with("/spec.saml") {
                continue;
            }
            let document = Document::new(uri.clone(), text.clone());
            for declaration in document.declarations {
                let kind = match declaration.kind.as_str() {
                    "boundry" => VocabularyKind::Boundary,
                    "concept" => VocabularyKind::Concept,
                    _ => continue,
                };
                let mut chars = declaration.name.chars();
                let Some(first) = chars.next() else { continue };
                let word = first.to_lowercase().collect::<String>() + chars.as_str();
                vocabulary
                    .entries
                    .entry(word)
                    .or_default()
                    .push(VocabularyEntry {
                        kind,
                        uri: uri.clone(),
                        start: declaration.name_start,
                        end: declaration.name_end,
                    });
            }
        }
        for entries in vocabulary.entries.values_mut() {
            entries
                .sort_by(|left, right| left.uri.cmp(&right.uri).then(left.start.cmp(&right.start)));
        }
        vocabulary
    }

    pub fn kind_for(&self, word: &str) -> Option<u32> {
        match word {
            "boundry" | "concept" => Some(5),
            "rule" => Some(12),
            _ => self.entries.get(word).map(|entries| {
                if entries
                    .iter()
                    .all(|entry| entry.kind == VocabularyKind::Boundary)
                {
                    3
                } else {
                    5
                }
            }),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Identifier,
    Punctuation,
    String,
}

#[derive(Clone)]
pub(crate) struct Token {
    pub value: String,
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[derive(Clone)]
pub struct Declaration {
    pub name: String,
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub name_start: usize,
    pub name_end: usize,
    pub(crate) parent: Option<usize>,
    pub(crate) header_start: usize,
    pub(crate) header_end: usize,
}

#[derive(Clone)]
pub struct Document {
    pub uri: String,
    text: String,
    line_starts: Vec<usize>,
    tokens: Vec<Token>,
    pub declarations: Vec<Declaration>,
    pub diagnostics: Vec<Value>,
    vocabulary: Vocabulary,
}

impl Document {
    pub fn new(uri: String, text: String) -> Self {
        Self::with_vocabulary(uri, text, &Vocabulary::default())
    }

    pub fn with_vocabulary(uri: String, text: String, vocabulary: &Vocabulary) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        let mut document = Self {
            uri,
            text,
            line_starts,
            tokens: Vec::new(),
            declarations: Vec::new(),
            diagnostics: Vec::new(),
            vocabulary: vocabulary.clone(),
        };
        document.scan();
        document.parse_structure();
        document
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn position(&self, offset: usize) -> Value {
        let offset = offset.min(self.text.len());
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let character = self.text[self.line_starts[line]..offset]
            .encode_utf16()
            .count();
        json!({"line": line, "character": character})
    }

    pub fn offset(&self, line: u64, character: u64) -> usize {
        let Some(&start) = self.line_starts.get(line as usize) else {
            return self.text.len();
        };
        let end = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut units = character;
        let mut offset = start;
        for ch in self.text[start..end].chars() {
            let width = ch.len_utf16() as u64;
            if width > units {
                break;
            }
            units -= width;
            offset += ch.len_utf8();
        }
        offset
    }

    pub fn range(&self, start: usize, end: usize) -> Value {
        json!({"start": self.position(start), "end": self.position(end)})
    }

    pub fn identifier_at(&self, offset: usize) -> Option<&str> {
        self.tokens
            .iter()
            .find(|token| {
                token.kind == TokenKind::Identifier && token.start <= offset && offset < token.end
            })
            .map(|token| token.value.as_str())
    }

    pub fn property_key_at(&self, offset: usize) -> Option<(&str, &str)> {
        let (index, token) = self.tokens.iter().enumerate().find(|(_, token)| {
            token.kind == TokenKind::Identifier && token.start <= offset && offset < token.end
        })?;
        if self.tokens.get(index + 1)?.value != ":" {
            return None;
        }
        let owner = self
            .declarations
            .iter()
            .filter(|declaration| {
                declaration.header_end <= token.start && token.start < declaration.end
            })
            .max_by_key(|declaration| declaration.start)?;
        Some((&token.value, &owner.kind))
    }

    pub fn symbol_tree(&self) -> Vec<Value> {
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| declaration.parent.is_none())
            .map(|(index, _)| self.symbol(index))
            .collect()
    }

    pub fn semantic_tokens(&self) -> Vec<u32> {
        let mut data = Vec::new();
        let (mut previous_line, mut previous_character) = (0, 0);
        for (index, token) in self.tokens.iter().enumerate() {
            if token.kind != TokenKind::Identifier {
                continue;
            }
            let (token_type, modifiers) = if RESTRICTED_KEYWORDS.contains(&token.value.as_str()) {
                (0, 0)
            } else if self.declarations.iter().any(|declaration| {
                declaration.name_start == token.start
                    && declaration.name_end == token.end
                    && matches!(declaration.kind.as_str(), "boundry" | "concept")
            }) {
                (1, 1)
            } else if token.value.chars().next().is_some_and(char::is_lowercase)
                && !self
                    .declarations
                    .iter()
                    .any(|declaration| declaration.start == token.start)
                && self
                    .tokens
                    .get(index + 1)
                    .is_some_and(|next| matches!(next.value.as_str(), ":" | "{"))
            {
                let declared = self.declarations.iter().any(|declaration| {
                    declaration.kind == "concept"
                        && declaration.header_end <= token.start
                        && token.start < declaration.end
                        && self.tokens[index + 1].value == "{"
                });
                (2, u32::from(declared))
            } else if self.vocabulary.entries.contains_key(&token.value) {
                (1, 0)
            } else {
                continue;
            };
            let position = self.position(token.start);
            let line = position["line"].as_u64().unwrap() as u32;
            let character = position["character"].as_u64().unwrap() as u32;
            let start = if line == previous_line {
                character - previous_character
            } else {
                character
            };
            let length = token.value.encode_utf16().count() as u32;
            data.extend([line - previous_line, start, length, token_type, modifiers]);
            previous_line = line;
            previous_character = character;
        }
        data
    }

    fn symbol(&self, index: usize) -> Value {
        let declaration = &self.declarations[index];
        let children: Vec<Value> = self
            .declarations
            .iter()
            .enumerate()
            .filter(|(_, child)| child.parent == Some(index))
            .map(|(child_index, _)| self.symbol(child_index))
            .collect();
        json!({
            "name": declaration.name,
            "detail": declaration.kind,
            "kind": self.vocabulary.kind_for(&declaration.kind).unwrap_or(1),
            "range": self.range(declaration.start, declaration.end),
            "selectionRange": self.range(declaration.name_start, declaration.name_end),
            "children": children,
        })
    }

    fn diagnose(&mut self, start: usize, end: usize, message: &str) {
        self.diagnostics.push(json!({
            "range": self.range(start, end),
            "severity": 1,
            "source": "saml",
            "message": message,
        }));
    }

    fn scan(&mut self) {
        let text = self.text.clone();
        let mut index = 0;
        while index < text.len() {
            let start = index;
            let ch = text[index..].chars().next().unwrap();
            if ch.is_whitespace() {
                index += ch.len_utf8();
                continue;
            }
            if text[index..].starts_with("//") {
                index = text[index..]
                    .find('\n')
                    .map(|next| index + next)
                    .unwrap_or(text.len());
                continue;
            }
            if ch == '"' || ch == '\'' {
                index += ch.len_utf8();
                let mut terminated = false;
                while index < text.len() {
                    let current = text[index..].chars().next().unwrap();
                    if current == ch {
                        index += current.len_utf8();
                        terminated = true;
                        break;
                    }
                    if current == '\n' {
                        break;
                    }
                    if current == '\\' {
                        index += current.len_utf8();
                        if index < text.len() {
                            index += text[index..].chars().next().unwrap().len_utf8();
                        }
                    } else {
                        index += current.len_utf8();
                    }
                }
                if !terminated {
                    self.diagnose(start, index, "Unterminated string");
                }
                self.tokens.push(Token {
                    value: text[start..index].into(),
                    start,
                    end: index,
                    kind: TokenKind::String,
                });
                continue;
            }
            if ch.is_alphabetic() || ch == '_' {
                index += ch.len_utf8();
                while index < text.len() {
                    let current = text[index..].chars().next().unwrap();
                    if !current.is_alphanumeric() && current != '_' {
                        break;
                    }
                    index += current.len_utf8();
                }
                self.tokens.push(Token {
                    value: text[start..index].into(),
                    start,
                    end: index,
                    kind: TokenKind::Identifier,
                });
                continue;
            }
            index += ch.len_utf8();
            self.tokens.push(Token {
                value: ch.to_string(),
                start,
                end: index,
                kind: TokenKind::Punctuation,
            });
        }
    }

    fn parse_structure(&mut self) {
        let mut brackets: Vec<(char, usize, usize)> = Vec::new();
        let mut blocks: Vec<Option<usize>> = Vec::new();
        let mut segment: Vec<usize> = Vec::new();
        for index in 0..self.tokens.len() {
            let token = self.tokens[index].clone();
            if self.vocabulary.entries.contains_key("state")
                && token.kind == TokenKind::Identifier
                && token.value == "state"
                && self
                    .tokens
                    .get(index + 1)
                    .is_some_and(|next| next.value == ":")
                && self
                    .tokens
                    .get(index + 2)
                    .is_some_and(|next| next.kind == TokenKind::Identifier)
            {
                let name = &self.tokens[index + 2];
                self.declarations.push(Declaration {
                    name: name.value.clone(),
                    kind: "state".into(),
                    start: token.start,
                    end: name.end,
                    name_start: name.start,
                    name_end: name.end,
                    parent: blocks.iter().rev().find_map(|item| *item),
                    header_start: token.start,
                    header_end: name.end,
                });
            }
            if token.kind != TokenKind::Punctuation {
                segment.push(index);
                continue;
            }
            match token.value.as_str() {
                "{" | "[" | "(" => {
                    brackets.push((token.value.chars().next().unwrap(), token.start, token.end));
                    if token.value == "{" {
                        let declaration = self.declaration_for(&segment, token.end, &blocks);
                        blocks.push(declaration);
                        segment.clear();
                    } else {
                        segment.push(index);
                    }
                }
                "}" | "]" | ")" => {
                    let expected = match token.value.as_str() {
                        "}" => '{',
                        "]" => '[',
                        _ => '(',
                    };
                    if brackets.last().is_some_and(|(ch, _, _)| *ch == expected) {
                        brackets.pop();
                    } else {
                        self.diagnose(
                            token.start,
                            token.end,
                            &format!("Unexpected '{}'", token.value),
                        );
                    }
                    if token.value == "}" {
                        if let Some(Some(declaration)) = blocks.pop() {
                            self.declarations[declaration].end = token.end;
                        }
                        segment.clear();
                    } else {
                        segment.push(index);
                    }
                }
                ";" => segment.clear(),
                _ => segment.push(index),
            }
        }
        for (ch, start, end) in brackets {
            self.diagnose(start, end, &format!("Unclosed '{ch}'"));
        }
    }

    fn declaration_for(
        &mut self,
        segment: &[usize],
        end: usize,
        blocks: &[Option<usize>],
    ) -> Option<usize> {
        let pair = segment
            .windows(2)
            .filter_map(|window| {
                let keyword = &self.tokens[window[0]];
                let name = &self.tokens[window[1]];
                (keyword.kind == TokenKind::Identifier
                    && name.kind == TokenKind::Identifier
                    && keyword.value.chars().next().is_some_and(char::is_lowercase)
                    && !matches!(keyword.value.as_str(), "extends" | "uses" | "via")
                    && name.value.chars().next().is_some_and(char::is_uppercase))
                .then_some((keyword, name))
            })
            .next_back()?;
        let index = self.declarations.len();
        self.declarations.push(Declaration {
            name: pair.1.value.clone(),
            kind: pair.0.value.clone(),
            start: pair.0.start,
            end,
            name_start: pair.1.start,
            name_end: pair.1.end,
            parent: blocks.iter().rev().find_map(|item| *item),
            header_start: self.tokens[segment[0]].start,
            header_end: end - 1,
        });
        Some(index)
    }
}
