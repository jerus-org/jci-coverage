//! Read the methods out of a Cobertura report and write new complexity values
//! back into it, leaving everything else about the file as it was.
//!
//! The reader and writer share one event stream, and only the elements that
//! change are rebuilt, so the XML declaration, the DOCTYPE, the indentation and
//! every untouched element come out byte for byte as they went in.

use std::borrow::Cow;

use anyhow::{Context, Result, bail};
use quick_xml::{
    Reader, Writer, XmlVersion,
    events::{BytesStart, BytesText, Event, attributes::Attribute},
};

/// A `<class>`: one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Class {
    /// Path of the source file, as the report gives it.
    pub(crate) filename: String,
    /// Index of the `<package>` it sits in.
    pub(crate) package: usize,
    /// The file's own `<line>`s as (line number, hits), in document order: every
    /// executable line `cargo llvm-cov` counted, not just where a method starts.
    pub(crate) lines: Vec<(u32, u64)>,
}

/// A `<method>`: one function, closure or instantiation of a generic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Method {
    /// Index of its `<class>`.
    pub(crate) class: usize,
    /// Demangled name, with entities decoded.
    pub(crate) name: String,
    /// Start line: the one `<line>` the method carries.
    pub(crate) line: u32,
    pub(crate) hits: u64,
}

/// What [`parse`] reads, all in document order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub(crate) package_count: usize,
    pub(crate) classes: Vec<Class>,
    pub(crate) methods: Vec<Method>,
}

impl Report {
    /// Fail unless this is a Rust report written by `cargo llvm-cov`.
    ///
    /// Complexity is computed from Rust source and joined to the report's
    /// functions by the names and start lines `cargo llvm-cov` gives them, so a
    /// report from another language or tool cannot be annotated: it would at
    /// best match nothing, and at worst match the wrong functions.
    pub(crate) fn require_rust_llvm_cov(&self) -> Result<()> {
        if self.classes.is_empty() {
            bail!("the report names no source files; `complexity` needs a Rust report");
        }
        if let Some(other) = self.classes.iter().find(|c| !c.filename.ends_with(".rs")) {
            bail!(
                "{} is not a Rust source file: `complexity` only supports Rust, \
                 with a Cobertura report written by `cargo llvm-cov`",
                other.filename
            );
        }
        // `cargo llvm-cov` names every function by its demangled Rust path.
        if self.methods.is_empty() || self.methods.iter().any(|m| !m.name.contains("::")) {
            bail!(
                "the report does not have the per-function entries `cargo llvm-cov` \
                 writes; `complexity` only supports a Cobertura report from \
                 `cargo llvm-cov report --cobertura`"
            );
        }
        Ok(())
    }
}

/// What to do with one `<method>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MethodEdit {
    /// Remove the element.
    Drop,
    /// Set its complexity, and when `hits` is given, its single line's hits and
    /// its `line-rate` (`1` if the hits are above zero, else `0`).
    Set { complexity: u32, hits: Option<u64> },
}

/// Complexity totals for the elements that contain methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rollup {
    pub(crate) classes: Vec<u32>,
    pub(crate) packages: Vec<u32>,
    pub(crate) root: u32,
}

/// One edit per `<method>`, in document order, and optionally the roll-ups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub(crate) methods: Vec<MethodEdit>,
    pub(crate) rollup: Option<Rollup>,
}

fn is(e: &BytesStart<'_>, name: &str) -> bool {
    e.name().as_ref() == name
}

fn attribute(e: &BytesStart<'_>, key: &str) -> Result<Option<String>> {
    for a in e.attributes() {
        let a = a.context("malformed attribute")?;
        if a.key.as_ref() == key {
            let value = a
                .normalized_value(XmlVersion::Implicit1_0)
                .context("malformed attribute value")?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn number<T: std::str::FromStr>(e: &BytesStart<'_>, key: &str) -> Result<T> {
    attribute(e, key)?
        .with_context(|| format!("<{}> has no `{key}`", e.name().as_ref()))?
        .parse()
        .map_err(|_| anyhow::anyhow!("`{key}` is not a number"))
}

/// Copy of `e` with the value of each attribute in `set` replaced, or added at
/// the end if `e` has none. Every other attribute keeps its raw text.
fn with_attributes(e: &BytesStart<'_>, set: &[(&str, String)]) -> Result<BytesStart<'static>> {
    let name = e.name().as_ref().to_owned();
    let mut out = BytesStart::new(name);
    let mut seen = vec![false; set.len()];
    for a in e.attributes() {
        let a = a.context("malformed attribute")?;
        match set.iter().position(|(k, _)| a.key.as_ref() == *k) {
            Some(i) => {
                seen[i] = true;
                out.push_attribute((set[i].0, set[i].1.as_str()));
            }
            None => out.push_attribute(Attribute {
                key: a.key,
                value: Cow::Owned(a.value.into_owned()),
            }),
        }
    }
    for (i, (key, value)) in set.iter().enumerate() {
        if !seen[i] {
            out.push_attribute((*key, value.as_str()));
        }
    }
    Ok(out)
}

/// Read the classes and methods of a Cobertura report.
pub(crate) fn parse(xml: &str) -> Result<Report> {
    let mut reader = Reader::from_str(xml);
    let mut report = Report::default();
    let mut depth = 0usize;
    // The method being read, until its end tag, and whether its line is read.
    let mut method: Option<(Method, bool)> = None;

    loop {
        let event = reader.read_event().context("malformed XML")?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                if is(e, "package") {
                    report.package_count += 1;
                } else if is(e, "class") {
                    if report.package_count == 0 {
                        bail!("malformed report: a <class> is not inside a <package>");
                    }
                    report.classes.push(Class {
                        filename: attribute(e, "filename")?.context("<class> has no `filename`")?,
                        package: report.package_count - 1,
                        lines: Vec::new(),
                    });
                } else if is(e, "method") {
                    if report.classes.is_empty() {
                        bail!("malformed report: a <method> is not inside a <class>");
                    }
                    method = Some((
                        Method {
                            class: report.classes.len() - 1,
                            name: attribute(e, "name")?.context("<method> has no `name`")?,
                            line: 0,
                            hits: 0,
                        },
                        false,
                    ));
                } else if is(e, "line") {
                    if let Some((m, read)) = method.as_mut() {
                        if !*read {
                            m.line = number(e, "number")?;
                            m.hits = number(e, "hits")?;
                            *read = true;
                        }
                    } else if let Some(class) = report.classes.last_mut() {
                        class.lines.push((number(e, "number")?, number(e, "hits")?));
                    }
                }
                if matches!(event, Event::Empty(_)) && is(e, "method") {
                    report.methods.extend(method.take().map(|(m, _)| m));
                } else if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if e.name().as_ref() == "method" {
                    report.methods.extend(method.take().map(|(m, _)| m));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        bail!("malformed XML: {depth} element(s) are never closed");
    }
    Ok(report)
}

/// Write `edit` into `xml`.
pub(crate) fn rewrite(xml: &str, edit: &Edit) -> Result<String> {
    let report = parse(xml)?;
    if edit.methods.len() != report.methods.len() {
        bail!(
            "{} method edits for a report with {} methods",
            edit.methods.len(),
            report.methods.len()
        );
    }
    if let Some(r) = &edit.rollup
        && (r.classes.len() != report.classes.len() || r.packages.len() != report.package_count)
    {
        bail!("the roll-up does not match the report's classes and packages");
    }

    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    // Whitespace is held back one event so that dropping a method can take its
    // indentation with it.
    let mut pending: Option<BytesText<'static>> = None;
    let (mut package, mut class, mut next_method) = (0usize, 0usize, 0usize);
    let mut root_done = false;
    // Hits to write into the first `<line>` of the method being copied.
    let mut line_hits: Option<u64> = None;

    loop {
        let event = reader.read_event().context("malformed XML")?.into_owned();

        if let Event::Text(t) = &event
            && t.as_ref().bytes().all(|b| b.is_ascii_whitespace())
        {
            if let Some(held) = pending.replace(t.clone()) {
                writer.write_event(Event::Text(held))?;
            }
            continue;
        }

        let out = match &event {
            Event::Start(e) | Event::Empty(e) if is(e, "method") => {
                let method_edit = &edit.methods[next_method];
                next_method += 1;
                match method_edit {
                    MethodEdit::Drop => {
                        pending = None;
                        if matches!(event, Event::Start(_)) {
                            reader
                                .read_to_end(e.name())
                                .context("malformed XML inside a dropped method")?;
                        }
                        continue;
                    }
                    MethodEdit::Set { complexity, hits } => {
                        let mut set = vec![("complexity", complexity.to_string())];
                        // An empty element has no `<line>` to carry the hits.
                        line_hits = hits.filter(|_| matches!(event, Event::Start(_)));
                        if let Some(h) = hits {
                            set.push(("line-rate", u8::from(*h > 0).to_string()));
                        }
                        let rebuilt = with_attributes(e, &set)?;
                        Some(if matches!(event, Event::Start(_)) {
                            Event::Start(rebuilt)
                        } else {
                            Event::Empty(rebuilt)
                        })
                    }
                }
            }
            Event::Start(e) | Event::Empty(e) if is(e, "line") && line_hits.is_some() => {
                let rebuilt =
                    with_attributes(e, &[("hits", line_hits.take().unwrap_or(0).to_string())])?;
                Some(if matches!(event, Event::Start(_)) {
                    Event::Start(rebuilt)
                } else {
                    Event::Empty(rebuilt)
                })
            }
            Event::Start(e) | Event::Empty(e) => {
                let rollup = edit.rollup.as_ref();
                let total = if is(e, "coverage") && !root_done {
                    root_done = true;
                    rollup.map(|r| r.root)
                } else if is(e, "package") {
                    package += 1;
                    rollup.map(|r| r.packages[package - 1])
                } else if is(e, "class") {
                    class += 1;
                    rollup.map(|r| r.classes[class - 1])
                } else {
                    None
                };
                total
                    .map(|t| with_attributes(e, &[("complexity", t.to_string())]))
                    .transpose()?
                    .map(|rebuilt| {
                        if matches!(event, Event::Start(_)) {
                            Event::Start(rebuilt)
                        } else {
                            Event::Empty(rebuilt)
                        }
                    })
            }
            Event::End(e) if e.name().as_ref() == "method" => {
                line_hits = None;
                None
            }
            _ => None,
        };

        if let Some(held) = pending.take() {
            writer.write_event(Event::Text(held))?;
        }
        if matches!(event, Event::Eof) {
            break;
        }
        writer.write_event(out.unwrap_or(event))?;
    }

    String::from_utf8(writer.into_inner()).context("the rewritten report is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `cargo llvm-cov report --cobertura` writes: two packages, three
    /// classes, and one `<method>` per instantiation with a single start line.
    const SAMPLE: &str = r#"<?xml version="1.0" ?>
<!DOCTYPE coverage SYSTEM "https://cobertura.sourceforge.net/xml/coverage-04.dtd">
<coverage branch-rate="0" branches-covered="0" branches-valid="0" complexity="0" line-rate="0.5" lines-covered="3" lines-valid="6" timestamp="1" version="2.0.3">
    <sources>
        <source>/work</source>
    </sources>
    <packages>
        <package line-rate="0.5" branch-rate="0" name="crates.x.src" complexity="0">
            <classes>
                <class branch-rate="0" complexity="0" filename="crates/x/src/lib.rs" line-rate="0.5" name="crates.x.src.lib.rs">
                    <methods>
                        <method name="x::f::&lt;u8&gt;" signature="" complexity="0" line-rate="1" branch-rate="1">
                            <lines>
                                <line hits="2" number="3" branch="false"/>
                            </lines>
                        </method>
                        <method name="x::f::&lt;u16&gt;" signature="" complexity="0" line-rate="0" branch-rate="0">
                            <lines>
                                <line hits="0" number="3" branch="false"/>
                            </lines>
                        </method>
                        <method name="x::g" signature="" complexity="0" line-rate="1" branch-rate="1">
                            <lines>
                                <line hits="1" number="9" branch="false"/>
                            </lines>
                        </method>
                    </methods>
                    <lines>
                        <line hits="2" number="3" branch="false"/>
                        <line hits="0" number="4" branch="false"/>
                        <line hits="1" number="9" branch="false"/>
                    </lines>
                </class>
            </classes>
        </package>
        <package line-rate="0.5" branch-rate="0" name="crates.x.src.sub" complexity="0">
            <classes>
                <class branch-rate="0" complexity="0" filename="crates/x/src/sub/a.rs" line-rate="0.5" name="crates.x.src.sub.a.rs">
                    <methods>
                        <method name="x::sub::a::h" signature="" complexity="0" line-rate="0" branch-rate="0">
                            <lines>
                                <line hits="0" number="2" branch="false"/>
                            </lines>
                        </method>
                    </methods>
                    <lines>
                        <line hits="0" number="2" branch="false"/>
                    </lines>
                </class>
                <class branch-rate="0" complexity="0" filename="crates/x/src/sub/b.rs" line-rate="1" name="crates.x.src.sub.b.rs">
                    <methods/>
                    <lines>
                        <line hits="5" number="1" branch="false"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>
"#;

    fn keep(n: usize) -> Edit {
        Edit {
            methods: (0..n)
                .map(|_| MethodEdit::Set {
                    complexity: 0,
                    hits: None,
                })
                .collect(),
            rollup: None,
        }
    }

    #[test]
    fn parse_reads_classes_and_methods_in_document_order() {
        let r = parse(SAMPLE).expect("parses");
        let classes: Vec<_> = r
            .classes
            .iter()
            .map(|c| (c.filename.as_str(), c.package))
            .collect();
        assert_eq!(
            classes,
            vec![
                ("crates/x/src/lib.rs", 0),
                ("crates/x/src/sub/a.rs", 1),
                ("crates/x/src/sub/b.rs", 1)
            ]
        );
        assert_eq!(r.package_count, 2);
        let methods: Vec<_> = r
            .methods
            .iter()
            .map(|m| (m.class, m.name.as_str(), m.line, m.hits))
            .collect();
        assert_eq!(
            methods,
            vec![
                (0, "x::f::<u8>", 3, 2),
                (0, "x::f::<u16>", 3, 0),
                (0, "x::g", 9, 1),
                (1, "x::sub::a::h", 2, 0),
            ]
        );
    }

    #[test]
    fn an_edit_that_changes_nothing_leaves_the_file_byte_identical() {
        let out = rewrite(SAMPLE, &keep(4)).expect("rewrites");
        assert_eq!(out, SAMPLE);
    }

    #[test]
    fn rewrite_sets_each_methods_complexity() {
        let edit = Edit {
            methods: [4, 5, 6, 7]
                .into_iter()
                .map(|complexity| MethodEdit::Set {
                    complexity,
                    hits: None,
                })
                .collect(),
            rollup: None,
        };
        let out = rewrite(SAMPLE, &edit).expect("rewrites");
        let r = parse(&out).expect("still parses");
        assert_eq!(r.methods.len(), 4);
        let values: Vec<_> = out
            .lines()
            .filter(|l| l.trim_start().starts_with("<method "))
            .map(|l| {
                l.split("complexity=\"")
                    .nth(1)
                    .and_then(|r| r.split('"').next())
                    .unwrap()
            })
            .collect();
        assert_eq!(values, vec!["4", "5", "6", "7"]);
        // Everything outside the complexity values is unchanged.
        assert_eq!(
            out.replace("complexity=\"4\"", "complexity=\"0\"")
                .replace("complexity=\"5\"", "complexity=\"0\"")
                .replace("complexity=\"6\"", "complexity=\"0\"")
                .replace("complexity=\"7\"", "complexity=\"0\""),
            SAMPLE
        );
    }

    #[test]
    fn a_dropped_method_disappears_with_its_lines_and_indentation() {
        let mut edit = keep(4);
        edit.methods[1] = MethodEdit::Drop;
        let out = rewrite(SAMPLE, &edit).expect("rewrites");
        assert!(!out.contains("x::f::&lt;u16&gt;"));
        assert!(out.contains("x::f::&lt;u8&gt;") && out.contains("x::g"));
        assert_eq!(parse(&out).expect("parses").methods.len(), 3);
        // No blank line is left where it was.
        assert!(!out.contains("\n\n"), "{out}");
        // The class's own line list is untouched.
        assert_eq!(
            out.matches("<line hits=").count(),
            SAMPLE.matches("<line hits=").count() - 1
        );
    }

    #[test]
    fn merged_hits_are_written_to_the_method_and_its_line() {
        let mut edit = keep(4);
        edit.methods[0] = MethodEdit::Set {
            complexity: 2,
            hits: Some(7),
        };
        edit.methods[3] = MethodEdit::Set {
            complexity: 1,
            hits: Some(0),
        };
        let out = rewrite(SAMPLE, &edit).expect("rewrites");
        let first = out
            .lines()
            .position(|l| l.contains("x::f::&lt;u8&gt;"))
            .unwrap();
        let method_line = out.lines().nth(first).unwrap();
        assert!(method_line.contains("line-rate=\"1\""), "{method_line}");
        assert!(
            out.lines()
                .nth(first + 2)
                .unwrap()
                .contains("hits=\"7\" number=\"3\"")
        );
        let last = out
            .lines()
            .position(|l| l.contains("x::sub::a::h"))
            .unwrap();
        assert!(out.lines().nth(last).unwrap().contains("line-rate=\"0\""));
        assert!(
            out.lines()
                .nth(last + 2)
                .unwrap()
                .contains("hits=\"0\" number=\"2\"")
        );
    }

    #[test]
    fn rollup_sets_class_package_and_root_complexity() {
        let mut edit = keep(4);
        edit.rollup = Some(Rollup {
            classes: vec![11, 22, 33],
            packages: vec![44, 55],
            root: 99,
        });
        let out = rewrite(SAMPLE, &edit).expect("rewrites");
        assert!(out.contains(
            r#"<coverage branch-rate="0" branches-covered="0" branches-valid="0" complexity="99""#
        ));
        assert!(out.contains(
            r#"<package line-rate="0.5" branch-rate="0" name="crates.x.src" complexity="44">"#
        ));
        assert!(out.contains(r#"name="crates.x.src.sub" complexity="55">"#));
        assert!(
            out.contains(
                r#"<class branch-rate="0" complexity="11" filename="crates/x/src/lib.rs""#
            )
        );
        assert!(out.contains(
            r#"<class branch-rate="0" complexity="33" filename="crates/x/src/sub/b.rs""#
        ));
    }

    #[test]
    fn rewrite_checks_the_edit_matches_the_report() {
        assert!(rewrite(SAMPLE, &keep(3)).is_err(), "too few method edits");
        assert!(rewrite(SAMPLE, &keep(5)).is_err(), "too many method edits");
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(parse("<coverage><package></coverage>").is_err());
        assert!(rewrite("<coverage><package></coverage>", &keep(0)).is_err());
    }

    #[test]
    fn a_report_without_methods_parses_to_nothing() {
        let r = parse("<coverage><packages/></coverage>").expect("parses");
        assert!(r.methods.is_empty() && r.classes.is_empty());
    }

    #[test]
    fn a_class_outside_a_package_or_a_method_outside_a_class_is_an_error() {
        let err = parse(r#"<coverage><class filename="a.rs"/></coverage>"#).unwrap_err();
        assert!(err.to_string().contains("<package>"), "{err}");
        let err =
            parse(r#"<coverage><package><method name="f"/></package></coverage>"#).unwrap_err();
        assert!(err.to_string().contains("<class>"), "{err}");
    }

    #[test]
    fn merged_hits_of_an_empty_method_do_not_leak_into_the_next_line() {
        let xml = r#"<coverage><packages><package><classes><class filename="a.rs"><methods><method name="f"/></methods><lines><line hits="1" number="1"/></lines></class></classes></package></packages></coverage>"#;
        let edit = Edit {
            methods: vec![MethodEdit::Set {
                complexity: 1,
                hits: Some(5),
            }],
            rollup: None,
        };
        let out = rewrite(xml, &edit).expect("rewrites");
        assert!(out.contains(r#"<line hits="1" number="1"/>"#), "{out}");
    }

    fn report_of(files: &[&str], methods: &[&str]) -> Report {
        Report {
            package_count: 1,
            classes: files
                .iter()
                .map(|f| Class {
                    filename: (*f).to_string(),
                    package: 0,
                    lines: Vec::new(),
                })
                .collect(),
            methods: methods
                .iter()
                .map(|n| Method {
                    class: 0,
                    name: (*n).to_string(),
                    line: 1,
                    hits: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn a_rust_llvm_cov_report_is_accepted() {
        let r = report_of(&["src/lib.rs"], &["x::f", "<x::S as x::T>::g"]);
        assert!(r.require_rust_llvm_cov().is_ok());
    }

    #[test]
    fn a_report_for_another_language_is_rejected() {
        let r = report_of(&["src/app.py"], &["pkg.mod.func"]);
        let err = r.require_rust_llvm_cov().unwrap_err().to_string();
        assert!(err.contains("app.py") && err.contains("Rust"), "{err}");
    }

    #[test]
    fn one_non_rust_file_among_rust_files_rejects_the_report() {
        let r = report_of(&["src/lib.rs", "web/app.js"], &["x::f"]);
        let err = r.require_rust_llvm_cov().unwrap_err().to_string();
        assert!(err.contains("web/app.js"), "{err}");
    }

    #[test]
    fn a_report_without_rust_function_names_is_rejected() {
        // Rust files, but no per-function data: not what `cargo llvm-cov` writes.
        let none = report_of(&["src/lib.rs"], &[]);
        let err = none.require_rust_llvm_cov().unwrap_err().to_string();
        assert!(err.contains("cargo llvm-cov"), "{err}");
        let plain = report_of(&["src/lib.rs"], &["doThing"]);
        assert!(plain.require_rust_llvm_cov().is_err());
    }

    #[test]
    fn a_report_with_no_classes_is_rejected() {
        let err = report_of(&[], &[]).require_rust_llvm_cov().unwrap_err();
        assert!(err.to_string().contains("no source files"), "{err}");
    }

    #[test]
    fn parse_reads_the_class_level_lines_but_not_a_methods_own_line() {
        let xml = r#"<coverage><packages><package><classes>
<class filename="src/a.rs"><methods>
<method name="x::f"><lines><line number="3" hits="9"/></lines></method>
</methods><lines>
<line number="3" hits="9" branch="false"/>
<line number="4" hits="0" branch="false"/>
<line number="5" hits="2" branch="false"/>
</lines></class>
<class filename="src/b.rs"><methods/><lines><line number="1" hits="1"/></lines></class>
</classes></package></packages></coverage>"#;
        let r = parse(xml).expect("parses");
        assert_eq!(r.classes[0].lines, vec![(3, 9), (4, 0), (5, 2)]);
        assert_eq!(r.classes[1].lines, vec![(1, 1)]);
        assert_eq!((r.methods[0].line, r.methods[0].hits), (3, 9));
    }
}
