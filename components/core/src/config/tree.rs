//! The document as nodes, built from the parser's events without recursion, so that nesting costs
//! no stack. Building stops at what makes the text unreadable as one document: a syntax error, a
//! second document, nesting too deep, an alias to nothing or a repeated key.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use fvm_contract::app_internal::problem::{ConfigProblem, ConfigProblemKind, LineColumn};
use saphyr_parser::{Event, Parser, ScalarStyle, ScanError};

use super::text::location;

/// Block nesting deeper than this is refused; the parser itself stops flow nesting sooner.
const MAX_NESTING: usize = 1000;

pub(super) struct Tree<'a> {
    nodes: Vec<Node<'a>>,
    root: Option<usize>,
}

pub(super) struct Node<'a> {
    pub kind: Kind<'a>,
    /// Where the node starts, at its anchor or tag if it has one. An alias is located at the node
    /// it refers to.
    pub location: LineColumn,
    /// Has an anchor, or is an alias.
    pub anchored: bool,
    pub tag: Option<String>,
}

pub(super) enum Kind<'a> {
    Scalar {
        text: Cow<'a, str>,
        style: ScalarStyle,
    },
    Sequence(Vec<usize>),
    /// Keys and values in file order.
    Mapping(Vec<(usize, usize)>),
    Alias,
}

impl<'a> Tree<'a> {
    pub fn root(&self) -> Option<&Node<'a>> {
        self.root.map(|index| &self.nodes[index])
    }

    pub fn node(&self, index: usize) -> &Node<'a> {
        &self.nodes[index]
    }
}

/// A collection whose end has not come yet.
struct Open {
    index: usize,
    /// A mapping's key waiting for its value.
    key: Option<usize>,
}

pub(super) fn build(text: &str) -> Result<Tree<'_>, ConfigProblem> {
    let mut nodes: Vec<Node<'_>> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut anchors: HashMap<usize, LineColumn> = HashMap::new();
    let mut root = None;
    let mut documents = 0;
    for item in Parser::new_from_str(text) {
        let (event, span) = item.map_err(scan_problem)?;
        let location = location(&span.start);
        let node = match event {
            Event::Nothing | Event::StreamStart | Event::DocumentEnd => continue,
            Event::StreamEnd => break,
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(problem(ConfigProblemKind::MultipleDocuments {}, location));
                }
                continue;
            }
            Event::SequenceStart(anchor, ref tag) | Event::MappingStart(anchor, ref tag) => {
                if open.len() >= MAX_NESTING {
                    return Err(problem(ConfigProblemKind::NestingTooDeep {}, location));
                }
                let kind = if matches!(event, Event::SequenceStart(..)) {
                    Kind::Sequence(Vec::new())
                } else {
                    Kind::Mapping(Vec::new())
                };
                if anchor != 0 {
                    anchors.insert(anchor, location);
                }
                open.push(Open {
                    index: nodes.len(),
                    key: None,
                });
                nodes.push(Node {
                    kind,
                    location,
                    anchored: anchor != 0,
                    tag: tag
                        .as_ref()
                        .map(|tag| format!("{}{}", tag.handle, tag.suffix)),
                });
                continue;
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let closed = open
                    .pop()
                    .expect("the parser ends only collections it started");
                if let Kind::Mapping(pairs) = &nodes[closed.index].kind {
                    check_repeated_keys(&nodes, pairs)?;
                }
                closed.index
            }
            Event::Alias(anchor) => {
                nodes.push(Node {
                    kind: Kind::Alias,
                    location: anchors[&anchor],
                    anchored: true,
                    tag: None,
                });
                nodes.len() - 1
            }
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 {
                    anchors.insert(anchor, location);
                }
                nodes.push(Node {
                    kind: Kind::Scalar { text, style },
                    location,
                    anchored: anchor != 0,
                    tag: tag.map(|tag| format!("{}{}", tag.handle, tag.suffix)),
                });
                nodes.len() - 1
            }
        };
        let Some(parent) = open.last_mut() else {
            root = Some(node);
            continue;
        };
        match &mut nodes[parent.index].kind {
            Kind::Sequence(items) => items.push(node),
            Kind::Mapping(pairs) => match parent.key.take() {
                None => parent.key = Some(node),
                Some(key) => pairs.push((key, node)),
            },
            Kind::Scalar { .. } | Kind::Alias => unreachable!("only collections are open"),
        }
    }
    Ok(Tree { nodes, root })
}

/// A key written twice in one mapping: the alphabetically first such key, at its first appearance.
/// Keys are compared by their text alone, so `1` and `"1"` are the same key.
fn check_repeated_keys(nodes: &[Node<'_>], pairs: &[(usize, usize)]) -> Result<(), ConfigProblem> {
    let mut first_seen: HashMap<&str, LineColumn> = HashMap::new();
    let mut reported: Option<(&str, LineColumn)> = None;
    for &(key, _) in pairs {
        let Some(text) = scalar_text(&nodes[key]) else {
            continue;
        };
        match first_seen.entry(text) {
            Entry::Vacant(entry) => {
                entry.insert(nodes[key].location);
            }
            Entry::Occupied(entry) => {
                if reported.is_none_or(|(chosen, _)| text < chosen) {
                    reported = Some((text, *entry.get()));
                }
            }
        }
    }
    match reported {
        Some((key, location)) => Err(problem(
            ConfigProblemKind::DuplicateKey {
                key: key.to_owned(),
            },
            location,
        )),
        None => Ok(()),
    }
}

pub(super) fn scalar_text<'n>(node: &'n Node<'_>) -> Option<&'n str> {
    match &node.kind {
        Kind::Scalar { text, .. } => Some(text),
        _ => None,
    }
}

fn scan_problem(error: ScanError) -> ConfigProblem {
    let kind = match error.info() {
        "while parsing node, found unknown anchor" => ConfigProblemKind::AnchorOrAlias {},
        "recursion limit exceeded" => ConfigProblemKind::NestingTooDeep {},
        info => ConfigProblemKind::Syntax {
            problem: info.to_owned(),
        },
    };
    problem(kind, location(error.marker()))
}

fn problem(kind: ConfigProblemKind, location: LineColumn) -> ConfigProblem {
    ConfigProblem {
        kind,
        location: Some(location),
    }
}
