use std::collections::{BTreeMap, BTreeSet, VecDeque};

use gent_types::NormalizedProviderEvent;
use serde_json::Value;

use crate::public_protocol::PublicWireFact;

type ItemKey = (String, String, String);
type ScopeKey = (String, String);
const MAX_REASONING_ITEMS: usize = 4096;
const MAX_REASONING_PARTS: usize = 256;
const TEXT: u8 = 1;
const SUMMARY: u8 = 2;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ItemParts {
    emitted: BTreeSet<(u8, u64)>,
    completed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Scope {
    items: usize,
    last_block: Option<(String, u8, u64)>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ReasoningState {
    items: BTreeMap<ItemKey, ItemParts>,
    order: VecDeque<ItemKey>,
    scopes: BTreeMap<ScopeKey, Scope>,
}

impl ReasoningState {
    pub(super) fn receive(&mut self, frame: &Value) -> Option<Vec<PublicWireFact>> {
        let method = frame.get("method").and_then(Value::as_str)?;
        if method == "item/reasoning/summaryPartAdded" {
            return Some(Vec::new());
        }
        let channel = match method {
            "item/reasoning/textDelta" => TEXT,
            "item/reasoning/summaryTextDelta" => SUMMARY,
            "item/completed"
                if frame.pointer("/params/item/type").and_then(Value::as_str)
                    == Some("reasoning") =>
            {
                0
            }
            _ => return None,
        };
        let item = if channel == 0 {
            frame.pointer("/params/item/id")
        } else {
            frame.pointer("/params/itemId")
        };
        let Some(item) = item.and_then(Value::as_str).filter(|id| !id.is_empty()) else {
            return Some(diagnostic("uncorrelatedCodexReasoning"));
        };
        let (thread, turn) = (field(frame, "threadId"), field(frame, "turnId"));
        if [thread, turn, item]
            .iter()
            .any(|id| id.is_empty() || id.len() > 1024)
        {
            return Some(diagnostic("invalidCodexReasoningIdentity"));
        }
        let delta = if channel == 0 {
            None
        } else {
            let index_name = if channel == TEXT {
                "contentIndex"
            } else {
                "summaryIndex"
            };
            let Some(index) = frame
                .get("params")
                .and_then(|params| params.get(index_name))
                .and_then(Value::as_u64)
            else {
                return Some(diagnostic("malformedCodexReasoningPart"));
            };
            let Some(text) = frame
                .pointer("/params/delta")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            else {
                return Some(diagnostic("malformedCodexThinkingDelta"));
            };
            Some((index, text))
        };
        let key = (thread.into(), turn.into(), item.into());
        let mut facts = self.remember(&key);
        if let Some((index, text)) = delta {
            self.emit(&key, channel, index, text, &mut facts);
            return Some(facts);
        }
        if self.items[&key].completed {
            return Some(facts);
        }
        self.items.get_mut(&key).unwrap().completed = true;
        let item = &frame["params"]["item"];
        let text = parts(item.get("text"));
        let text = if text.is_empty() {
            parts(item.get("content"))
        } else {
            text
        };
        for (channel, values) in [(TEXT, text), (SUMMARY, parts(item.get("summary")))] {
            for (index, text) in values {
                if self.items[&key].emitted.contains(&(channel, index)) {
                    continue;
                }
                if !self.emit(&key, channel, index, &text, &mut facts) {
                    break;
                }
            }
        }
        Some(facts)
    }

    fn remember(&mut self, key: &ItemKey) -> Vec<PublicWireFact> {
        if self.items.contains_key(key) {
            return Vec::new();
        }
        let mut facts = Vec::new();
        if self.items.len() >= MAX_REASONING_ITEMS {
            if let Some(oldest) = self.order.pop_front() {
                self.items.remove(&oldest);
                let scope_key = (oldest.0, oldest.1);
                if let Some(scope) = self.scopes.get_mut(&scope_key) {
                    scope.items -= 1;
                    if scope.items == 0 {
                        self.scopes.remove(&scope_key);
                    }
                }
            }
            facts = diagnostic("codexReasoningItemLimit");
        }
        self.order.push_back(key.clone());
        self.items.insert(key.clone(), ItemParts::default());
        self.scopes
            .entry((key.0.clone(), key.1.clone()))
            .or_default()
            .items += 1;
        facts
    }

    fn emit(
        &mut self,
        key: &ItemKey,
        channel: u8,
        index: u64,
        text: &str,
        facts: &mut Vec<PublicWireFact>,
    ) -> bool {
        let item = self.items.get_mut(key).unwrap();
        if !item.emitted.contains(&(channel, index)) && item.emitted.len() >= MAX_REASONING_PARTS {
            facts.extend(diagnostic("codexReasoningPartLimit"));
            return false;
        }
        item.emitted.insert((channel, index));
        let scope = self
            .scopes
            .get_mut(&(key.0.clone(), key.1.clone()))
            .unwrap();
        let block = (key.2.clone(), channel, index);
        let separated = scope.last_block.as_ref().is_some_and(|last| last != &block);
        scope.last_block = Some(block);
        facts.push(PublicWireFact::Event(NormalizedProviderEvent::Thinking {
            text: if separated {
                format!("\n\n{text}")
            } else {
                text.into()
            },
            is_partial: true,
        }));
        true
    }
}

fn field<'a>(frame: &'a Value, name: &str) -> &'a str {
    frame
        .get("params")
        .and_then(|params| params.get(name))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn parts(value: Option<&Value>) -> Vec<(u64, String)> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => vec![(0, text.clone())],
        Some(Value::Array(values)) => values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                value
                    .as_str()
                    .or_else(|| value.get("text").and_then(Value::as_str))
                    .filter(|text| !text.is_empty())
                    .map(|text| (index as u64, text.into()))
            })
            .take(MAX_REASONING_PARTS + 1)
            .collect(),
        _ => Vec::new(),
    }
}

fn diagnostic(classification: &str) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    )]
}
