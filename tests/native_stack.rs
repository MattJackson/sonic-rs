//! Native DOM stack regressions run in child processes so an abort identifies its case.
#![cfg(not(target_family = "wasm"))]

use std::{collections::HashMap, process::Command};

use serde::Deserialize;
use sonic_rs::{self as json, Deserializer, JsonValueTrait, LazyValue, Read, Value};

const MAX_DEPTH: usize = 254;
const CHILD_CASE: &str = "SONIC_NATIVE_STACK_CASE";

fn nested(depth: usize, shape: usize, leaf: &str) -> String {
    let is_array = |level| shape == 0 || (shape == 2 && level % 2 == 0);
    let mut input = String::new();
    for level in 0..depth {
        input.push_str(if is_array(level) { "[" } else { r#"{"a":"# });
    }
    input.push_str(leaf);
    for level in (0..depth).rev() {
        input.push(if is_array(level) { ']' } else { '}' });
    }
    input
}

fn depth_error<T>(result: json::Result<T>) {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("accepted a materialized container beyond the depth budget"),
    };
    assert!(error.is_syntax());
    assert!(error
        .to_string()
        .contains("Encountered nesting of JSON maps and arrays"));
    assert!(error.line() > 0);
    assert!(error.column() > 0);
}

fn isolated(case: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native_stack_child", "--nocapture"])
        .env(CHILD_CASE, case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native stack case {case}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn native_stack_boundary() {
    isolated("boundary");
}

#[test]
fn native_stack_typed_ancestors() {
    isolated("typed");
}

#[test]
fn native_stack_lifecycle() {
    isolated("lifecycle");
}

#[test]
fn native_stack_error_cleanup() {
    isolated("cleanup");
}

#[test]
fn native_stack_deep_selective_reads() {
    isolated("selective");
}

#[test]
fn native_stack_child() {
    let Ok(case) = std::env::var(CHILD_CASE) else {
        return;
    };
    std::thread::Builder::new()
        .name(format!("native-stack-{case}"))
        // Deliberately independent of RUST_MIN_STACK and the test harness's stack.
        .stack_size(2 * 1024 * 1024)
        .spawn(move || match case.as_str() {
            "boundary" => boundary(),
            "typed" => typed_ancestors(),
            "lifecycle" => lifecycle(),
            "cleanup" => cleanup(),
            "selective" => selective(),
            _ => panic!("unknown native stack case"),
        })
        .unwrap()
        .join()
        .unwrap();
}

fn boundary() {
    for shape in 0..3 {
        for depth in [0, 1, 127, 128, 129, 253, MAX_DEPTH] {
            for leaf in ["0", "true", r#""escaped\nstring""#] {
                let compact = nested(depth, shape, leaf);
                let spaced = compact.replace('[', "[ \n ").replace('{', "{ \n ");
                for input in [compact, spaced] {
                    drop(json::from_str::<Value>(&input).unwrap());
                    drop(json::from_slice::<Value>(input.as_bytes()).unwrap());
                }
            }
        }
        for depth in [MAX_DEPTH + 1, MAX_DEPTH + 2, 2_000, 10_000] {
            let input = nested(depth, shape, "0");
            depth_error(json::from_str::<Value>(&input));
            depth_error(json::from_slice::<Value>(input.as_bytes()));
        }
    }
    depth_error(json::from_str::<Value>(&"[".repeat(10_000)));
    let stream = format!("{0} {0} {0}", nested(MAX_DEPTH, 2, "0"));
    let mut decoder = Deserializer::new(Read::from(stream.as_str()));
    for _ in 0..3 {
        drop(decoder.deserialize::<Value>().unwrap());
    }
    let siblings = vec![nested(MAX_DEPTH - 1, 2, "0"); 8].join(",");
    drop(json::from_str::<Value>(&format!("[{siblings}]")).unwrap());
}

fn typed_ancestors() {
    #[derive(Deserialize)]
    struct Envelope {
        inner: Vec<Value>,
    }
    #[derive(Deserialize)]
    struct Link {
        next: Option<Box<Link>>,
        value: Value,
    }

    for shape in 0..3 {
        let map = |depth| format!(r#"{{"a":{}}}"#, nested(depth, shape, "0"));
        drop(json::from_str::<HashMap<String, Value>>(&map(MAX_DEPTH - 1)).unwrap());
        depth_error(json::from_str::<HashMap<String, Value>>(&map(MAX_DEPTH)));
        let envelope = |depth| format!(r#"{{"inner":[{}]}}"#, nested(depth, shape, "0"));
        let parsed: Envelope = json::from_str(&envelope(MAX_DEPTH - 2)).unwrap();
        assert_eq!(parsed.inner.len(), 1);
        depth_error(json::from_str::<Envelope>(&envelope(MAX_DEPTH - 1)));

        // Several already-active typed visitors must lend the DOM only their remaining budget.
        for ancestors in [32, 128] {
            let linked = |depth| {
                format!(
                    r#"{}{{"next":null,"value":{}}}{}"#,
                    r#"{"next":"#.repeat(ancestors - 1),
                    nested(depth, shape, "0"),
                    r#", "value":null}"#.repeat(ancestors - 1),
                )
            };
            let chain: Link = json::from_str(&linked(MAX_DEPTH - ancestors)).unwrap();
            let mut leaf = &chain;
            while let Some(next) = &leaf.next {
                leaf = next;
            }
            assert!(!leaf.value.is_null());
            depth_error(json::from_str::<Link>(&linked(MAX_DEPTH - ancestors + 1)));
        }
    }
}

fn lifecycle() {
    for shape in 0..3 {
        let input = nested(MAX_DEPTH, shape, "0");
        let parsed: Value = json::from_str(&input).unwrap();
        let mut owned = parsed.clone();
        let mut cursor = &mut owned;
        for level in 0..MAX_DEPTH {
            cursor = if shape == 0 || (shape == 2 && level % 2 == 0) {
                &mut cursor[0]
            } else {
                &mut cursor["a"]
            };
        }
        *cursor = Value::from(7);
        assert_eq!(json::to_string(&parsed).unwrap(), input);
        assert_eq!(
            json::to_string(&owned).unwrap(),
            nested(MAX_DEPTH, shape, "7")
        );
        let converted: Value = json::from_value(&owned).unwrap();
        assert_eq!(
            json::to_string(&converted).unwrap(),
            nested(MAX_DEPTH, shape, "7")
        );
        drop(owned.clone());
        drop(converted);
        drop(owned);
        drop(parsed);
    }
}

fn cleanup() {
    for shape in 0..3 {
        // A rejected late branch must clean up already materialized siblings on this worker.
        let prefix = vec![r#"{"ready":[1,2,3],"text":"allocated\nstring"}"#; 1_024].join(",");
        let oversized = format!("[{prefix},{}]", nested(2_000, shape, "0"));
        for _ in 0..8 {
            depth_error(json::from_str::<Value>(&oversized));
            drop(json::from_str::<Value>("{\"next\":[7]}").unwrap());
        }
        for leaf in ["[1,]", r#"{"broken":}"#, r#""\uD800""#, "tru", "01"] {
            let input = nested(MAX_DEPTH - 1, shape, leaf);
            assert!(Deserializer::new(Read::from(input.as_str()))
                .deserialize::<Value>()
                .is_err());
            assert!(Deserializer::new(Read::from(input.as_bytes()))
                .deserialize::<Value>()
                .is_err());
            assert_eq!(json::from_str::<Value>("[7]").unwrap()[0].as_u64(), Some(7));
        }
        let input = format!("{} trailing", nested(MAX_DEPTH, shape, "0"));
        assert!(json::from_str::<Value>(&input).is_err());
        let mut invalid = nested(MAX_DEPTH, shape, r#""x""#).into_bytes();
        let at = invalid.iter().position(|&byte| byte == b'x').unwrap();
        invalid[at] = 0xff;
        assert!(Deserializer::new(Read::from(invalid.as_slice()))
            .deserialize::<Value>()
            .is_err());
        drop(
            Deserializer::new(Read::from(invalid.as_slice()))
                .utf8_lossy()
                .deserialize::<Value>()
                .unwrap(),
        );
        drop(json::from_str::<Value>("{}").unwrap());
    }
}

fn selective() {
    #[derive(Deserialize)]
    struct Envelope<'a> {
        #[serde(borrow)]
        data: LazyValue<'a>,
        wanted: u64,
    }
    for shape in 0..3 {
        let raw = nested(10_000, shape, "1e400");
        let input = format!(r#"{{"data":{raw},"ignored":{raw},"wanted":7}}"#);
        let parsed: Envelope<'_> = json::from_str(&input).unwrap();
        assert_eq!(parsed.data.as_raw_str(), raw);
        assert_eq!(parsed.wanted, 7);
        let mut paths = json::PointerTree::new();
        paths.add_path(["wanted"]);
        let selected = json::get_many(&input, &paths).unwrap();
        assert_eq!(selected[0].as_ref().unwrap().as_raw_str(), "7");
    }
}
