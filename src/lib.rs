//! The reference implementation for JSON-NS, a small and basic subset of JSON-LD. See the [blog
//! post] for what this is and why it exists.
//!
//!  [blog post]: https://stephank.nl/p/2018-10-20-a-proposal-for-standardising-a-subset-of-json-ld.html
//!
//! This implementation uses the `serde_json` crate types to represent JSON values. Doing basic
//! processing involves creating a `Processor`, which holds some optional configuration, and giving
//! it a `Value` to process:
//!
//! ```rust
//! #[macro_use]
//! extern crate serde_json as json;
//! extern crate json_ns;
//!
//! use json_ns::Processor;
//!
//! fn main() {
//!     // Some example input.
//!     let input = json!({
//!         "@context": {
//!             "foo": "http://example.com/ns#"
//!         },
//!         "foo:hello": "world"
//!     });
//!
//!     // Process the document, and use `bar` instead as the output prefix.
//!     let output = Processor::new()
//!         .add_rule("bar", "http://example.com/ns#")
//!         .process_value(&input);
//!
//!     // Check that the output is what we expected.
//!     assert_eq!(output, json!({
//!         "bar:hello": "world"
//!     }));
//! }
//! ```
//!
//! Without the processor configuration, this code can be even shorter:
//!
//! ```rust,ignore
//! let output = Processor::new().process_value(&input);
//! ```
//!
//! In this case, the output document contains a property named `http://example.com/ns#hello`.
//!
//! Often, the bulk of the properties you expect are in a single namespace. In this case, it may be
//! useful to set a default namespace on the output, for which properties are not prefixed at all:
//!
//! ```rust,ignore
//! processor.add_rule("", "http://example.com/ns#");
//! ```
//!
//! The output then contains a property named just `hello`. This is especially useful when passing
//! the value on to `serde_json::from_value` to parse it into a struct that derives `Deserialize`.
//!
//! Note that the output should *not* itself be considered a JSON-NS document. Running input
//! through a processor twice may produce unexpected results.
//!
//! That should cover the basics. More details can be found in the documentation of the structs,
//! fields and functions.

#[macro_use]
extern crate cfg_if;
extern crate serde_json as json;

cfg_if! {
    if #[cfg(test)] {
        extern crate colored;
        mod test;
    }
}

use json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::slice::Iter;

type Map = json::Map<String, Value>;

/// Iterator used to walk a value that may or may not be an array.
enum OneOrMany<'a> {
  None,
  One(&'a Value),
  Many(Iter<'a, Value>),
}

impl<'a> From<&'a Value> for OneOrMany<'a> {
    fn from(value: &'a Value) -> Self {
        match *value {
            Value::Array(ref arr) => OneOrMany::Many(arr.iter()),
            ref value => OneOrMany::One(value),
        }
    }
}

impl<'a> Iterator for OneOrMany<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<&'a Value> {
        match *self {
            OneOrMany::None => None,
            OneOrMany::One(value) => {
                *self = OneOrMany::None;
                Some(value)
            },
            OneOrMany::Many(ref mut iter) => {
                iter.next()
            },
        }
    }
}

/// Structure holding the current context to interpret a document with.
///
/// An instance of this struct is part of the `Processor`, which can be modified to provide an
/// external context to interpret documents with. Such a custom context can also be created from
/// JSON using one of the `From` implementations.
#[derive(Clone,Debug,Default)]
pub struct Context {
    /// The default namespace, for properties that are not a keyword, CURIE, or IRI.
    pub ns: Option<String>,
    /// The Default language for internationalised properties that don't specify one. The empty
    /// string when not defined.
    pub lang: String,
    /// Map of defined CURIE prefixes to their base IRIs.
    pub prefixes: BTreeMap<String, String>,
    /// Map of defined aliases by their literal property names.
    pub aliases: BTreeMap<String, String>,
    /// Map of defined container mappings by their literal property names.
    pub container: BTreeMap<String, String>,
}

impl Context {
    /// An alias for `Context::default()`.
    pub fn new() -> Context {
        Context::default()
    }

    /// Merge an `@context` value into this structure.
    pub fn merge_value(&mut self, value: &Value) {
        for value in OneOrMany::from(value) {
            match *value {
                Value::Null => {
                    // A null clears the context.
                    *self = Context::default();
                },
                Value::Object(ref object) => {
                    // An object is merged into the context.
                    self.merge_object(object);
                },
                _ => {
                    // Captures remote context references, but also anything else we don't understand.
                    // These are simply ignored.
                },
            }
        }
    }

    /// Merge an `@context` object into this structure.
    pub fn merge_object(&mut self, object: &Map) {
        for (key, value) in object {
            if is_keyword(key) {
                match key.as_str() {
                    "@vocab" => {
                        // Set the default namespace. May be null to clear it.
                        if let Some(ns) = value.as_str().filter(|s| is_absolute_iri(s)) {
                            self.ns = Some(ns.to_owned());
                        } else if value.is_null() {
                            self.ns = None;
                        }
                    },
                    "@language" => {
                        // Set the default language. May be null to clear it.
                        if let Some(lang) = value.as_str() {
                            self.lang = lang.to_owned();
                        } else if value.is_null() {
                            self.lang = "".to_owned();
                        }
                    },
                    _ => {},
                }
            } else {
                match *value {
                    Value::String(ref string) => {
                        // Define a namespace.
                        if is_curie_prefix(key) && is_absolute_iri(string) {
                            self.prefixes.insert(key.to_owned(), string.to_owned());
                        }
                    },
                    Value::Object(ref object) => {
                        // Look for an alias.
                        let alias = object.get("@id")
                            .and_then(Value::as_str)
                            .filter(|string| !is_keyword(string));
                        if let Some(alias) = alias {
                            self.aliases.insert(key.to_owned(), alias.to_owned());
                        }

                        // Look for a container mapping.
                        let container = object.get("@container")
                            .and_then(Value::as_str);
                        if let Some(container) = container {
                            self.container.insert(key.to_owned(), container.to_owned());
                        }
                    },
                    Value::Null => {
                        // A null value is used to clear whatever was defined.
                        self.prefixes.remove(key);
                        self.aliases.remove(key);
                        self.container.remove(key);
                    },
                    _ => {},
                }
            }
        }
    }

    /// Expand a name according to this context.
    ///
    /// A name may be an absolute IRI, a CURIE within a defined namespace, or a name in the default
    /// namespace, otherwise `None` is returned (and the property or value should be dropped).
    pub fn expand_name<'a>(&self, name: &'a str) -> Option<Cow<'a, str>> {
        if name.starts_with('@') {
            return None;
        }

        let mut parts = name.splitn(2, ':');
        let prefix = parts.next().unwrap();
        if let Some(suffix) = parts.next() {
            if let Some(base) = self.prefixes.get(prefix) {
                // A CURIE within a defined namespace.
                Some(Cow::from(format!("{}{}", base, suffix)))
            } else {
                // An absolute IRI in some other scheme.
                Some(Cow::from(name))
            }
        } else if let Some(ref base) = self.ns {
            // A term in the default namespace.
            Some(Cow::from(format!("{}{}", base, name)))
        } else {
            None
        }
    }
}

impl<'a> From<&'a Value> for Context {
    fn from(value: &'a Value) -> Context {
        let mut context = Context::default();
        context.merge_value(value);
        context
    }
}

impl<'a> From<&'a Map> for Context {
    fn from(object: &'a Map) -> Context {
        let mut context = Context::default();
        context.merge_object(object);
        context
    }
}

/// Structure holding the target context to reword a document to.
///
/// An instance of this struct is part of the `Processor`, which can be modified to provide rules
/// according to which the output will be reworded.
///
/// By default, this context is empty, which will result in an output document containing only
/// absolute IRIs.
#[derive(Clone,Debug,Default)]
pub struct TargetContext {
    /// Pairs of CURIE prefixes and their respective base IRIs.
    ///
    /// For absolute IRIs that are about to be added to the output document, the processor will try
    /// to find a matching prefix in this list. If found, a CURIE will be used instead.
    ///
    /// This list may also contain an entry with an empty string prefix, which then represents the
    /// default namespace of the output document.
    pub rules: Vec<(String, String)>,
}

impl TargetContext {
    /// Alias for `TargetContext::default()`.
    pub fn new() -> TargetContext {
        TargetContext::default()
    }

    /// A short-hand for adding a rule.
    pub fn add_rule(&mut self, prefix: &str, base: &str) -> &mut Self {
        self.rules.push((prefix.to_owned(), base.to_owned()));
        self
    }

    /// Compact an absolute IRI according to this context.
    pub fn compact_iri<'a>(&self, iri: &'a str) -> Cow<'a, str> {
        for (prefix, base) in &self.rules {
            if iri.starts_with(base) {
                let suffix = &iri[base.len()..];
                if prefix.is_empty() {
                    // Matched the default namespace.
                    return Cow::from(suffix);
                } else {
                    // Matched a prefix, generate a CURIE.
                    return Cow::from(format!("{}:{}", prefix, suffix));
                }
            }
        }
        // No match, output the absolute IRI.
        Cow::from(iri)
    }
}

/// A document processor.
///
/// This structure holds configuration for processing documents. The defaults are fine if the
/// output document should contain only absolute IRIs, but usually you want to set some namespaces
/// for the output document in the `TargetContext` contained within.
#[derive(Clone,Debug,Default)]
pub struct Processor {
    /// External context added to the document. Defaults to an empty context, so only inline
    /// contexts in the document itself are used.
    pub context: Context,
    /// Target context to reword the document to. Defaults to an empty context, so the result will
    /// contain only absolute IRIs for all properties and types.
    pub target: TargetContext,
}

impl Processor {
    /// Alias for `Processor::default()`.
    pub fn new() -> Processor {
        Processor::default()
    }

    /// A short-hand for adding a rule to the contained `TargetContext`.
    pub fn add_rule(&mut self, prefix: &str, base: &str) -> &mut Self {
        self.target.add_rule(prefix, base);
        self
    }

    /// Process a value, using the configuration in this struct.
    pub fn process_value(&self, value: &Value) -> Value {
        self.process_value_inner(value, &self.context)
    }

    /// Process an object, using the configuration in this struct.
    pub fn process_object(&self, object: &Map) -> Map {
        self.process_object_inner(object, &self.context)
    }

    /// Process a value with a local context.
    fn process_value_inner(&self, value: &Value, context: &Context) -> Value {
        match *value {
            Value::Array(ref array) => {
                let array = array.iter()
                    .map(|value| self.process_value_inner(value, context))
                    .collect::<Vec<_>>();
                Value::Array(array)
            },
            Value::Object(ref object) => {
                Value::Object(self.process_object_inner(object, context))
            },
            ref value => value.clone(),
        }
    }

    /// Process an object with a local context.
    fn process_object_inner(&self, object: &Map, context: &Context) -> Map {
        // Extend the active context with the local context, if present.
        let local_context = object.get("@context").map(|value| {
            let mut context = context.clone();
            context.merge_value(value);
            context
        });
        let context = local_context.as_ref().unwrap_or(context);

        let mut result = Map::with_capacity(object.len());
        for (key, value) in object {
            if key.starts_with('@') {
                // A keyword property.
                match key.as_str() {
                    "@id" => {
                        // Document ID, must be an absolute IRI.
                        if let Some(iri) = value.as_str().filter(|s| is_absolute_iri(s)) {
                            result.insert(key.clone(), Value::String(iri.to_owned()));
                        }
                    },
                    "@type" => {
                        // Document type, a string or array of strings, each of which expands to an
                        // absolute IRI. (We don't support `@type` on values, like JSON-LD.)
                        let value = OneOrMany::from(value)
                            .filter_map(|value| value.as_str())
                            .filter_map(|string| context.expand_name(string))
                            .map(|iri| self.target.compact_iri(&iri).into_owned())
                            .map(Value::String)
                            .collect::<Vec<_>>();
                        if !value.is_empty() {
                            result.insert(key.clone(), Value::Array(value));
                        }
                    },
                    _ => {
                        // Ignore `@context` (already processed) and other unrecognized keywords.
                    },
                }

                continue;
            }

            // Look for an alias.
            let resolved = context.aliases.get(key).map(String::as_str).unwrap_or(key);

            // Resolve in the current context.
            let resolved = match context.expand_name(resolved) {
                Some(iri) => self.target.compact_iri(&iri).into_owned(),
                None => continue,
            };

            // Look for a container mapping of the original property name.
            result.insert(resolved, match context.container.get(key).map(String::as_str) {
                Some("@language") => {
                    // An internationalised property.
                    match *value {
                        Value::String(_) => {
                            // Normalise a string value to a language map with a single entry for
                            // the context default language.
                            let mut object = Map::with_capacity(1);
                            object.insert(context.lang.clone(), value.clone());
                            Value::Object(object)
                        },
                        Value::Object(ref object) => {
                            // Filter non-string values from the object.
                            let object = object.iter()
                                .filter(|(_, value)| value.is_string())
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect();
                            Value::Object(object)
                        },
                        _ => {
                            // Drop unrecognised values.
                            continue;
                        },
                    }
                },
                _ => {
                    // No or unrecognized container mapping, which we treat as a normal value.
                    // Expand it by recursing.
                    self.process_value_inner(value, context)
                },
            });
        }

        result
    }
}

/// Whether the input is a keyword.
fn is_keyword(input: &str) -> bool {
    input.starts_with('@')
}

/// Whether the input is a valid absolute IRI.
fn is_absolute_iri(input: &str) -> bool {
    input.contains(':') && !input.starts_with('@')
}

/// Whether the input is a valid CURIE prefix.
fn is_curie_prefix(input: &str) -> bool {
    !input.is_empty() && !input.contains(':') && !input.starts_with('@')
}
