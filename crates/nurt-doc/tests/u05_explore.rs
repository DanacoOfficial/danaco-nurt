// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use saphyr::{LoadableYamlNode, MarkedYaml, YamlData};

fn dump(node: &MarkedYaml, text: &str, depth: usize, out: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let s = node.span.start.index();
    let e = node.span.end.index().min(chars.len());
    let slice: String = chars[s.min(e)..e].iter().collect();
    let kind = match &node.data {
        YamlData::Sequence(_) => "SEQ".to_owned(),
        YamlData::Mapping(_) => "MAP".to_owned(),
        YamlData::Value(v) => format!("VAL {v:?}"),
        YamlData::Tagged(t, _) => format!("TAG {t:?}"),
        YamlData::Alias(a) => format!("ALI {a}"),
        YamlData::BadValue => "BAD".to_owned(),
        YamlData::Representation(..) => "REP".to_owned(),
    };
    out.push_str(&format!(
        "{}{kind} [{s}..{}] {:?}\n",
        "  ".repeat(depth),
        node.span.end.index(),
        slice
    ));
    match &node.data {
        YamlData::Sequence(items) => items.iter().for_each(|i| dump(i, text, depth + 1, out)),
        YamlData::Mapping(map) => map.iter().for_each(|(k, v)| {
            dump(k, text, depth + 1, out);
            dump(v, text, depth + 1, out);
        }),
        YamlData::Tagged(_, inner) => dump(inner, text, depth + 1, out),
        _ => {}
    }
}

#[test]
fn explore_spans() {
    let samples = [
        "a: 1\nb: 'x'\nc: \"y\"\n",
        "k: &anc !!str v\nr: *anc\n",
        "lit: |+\n  one\n  two\n\n\nnext: 2\n",
        "fold: >-\n  a\n  b\n# c\nz: 1\n",
        "m: &m\n  x: 1\n  y: [1, 2]\nn: !t {p: q, r: s}\n",
        "l:\n- a\n- b\n  # c\nlast:\n",
        "plain: multi\n  line\n  text\nq: \"two\n  lines\"\n",
        "? complex\n: value\n--- \n- doc2\n...\n",
        "e:\nf: ~\ng: []\n",
        "zażółć: ąę\nx: 1\n",
        "- &s [a]\n- !!map {}\n- &e\n- *s\n",
        "",
        "--- |\n  only\n",
        "--- !t &a\n- x\n",
    ];
    let mut out = String::new();
    for text in samples {
        out.push_str(&format!("=== {text:?}\n"));
        match MarkedYaml::load_from_str(text) {
            Ok(docs) => docs.iter().for_each(|d| dump(d, text, 0, &mut out)),
            Err(e) => out.push_str(&format!("ERR {e}\n")),
        }
    }
    eprintln!("{out}");
}
