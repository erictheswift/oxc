//! A language server lints in one process per request: with `free_programs`, a run gives its programs back, so the
//! arena reservation stays flat however many requests come.

use std::sync::mpsc;

use oxc_linter_tsrs::linter::{Options, Sink, spawn};
use oxc_linter_tsrs::protocol::{HeadlessConfig, HeadlessRule, Output, Payload};
use tsrs_core::reserve::reserved_in_use;

fn project() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrslint-repeated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "es2020", "lib": ["es2020", "dom"], "strict": true } }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("a.ts"),
        "import { load } from './b';\nexport async function f() { return load(); }\nf();\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.ts"),
        "export async function load(): Promise<number[]> { return [1, 2, 3].map((x) => x * 2); }\n",
    )
    .unwrap();
    dir.canonicalize().unwrap()
}

fn diagnostics(dir: &std::path::Path, opts: &Options) -> Vec<String> {
    let payload = Payload {
        version: 2,
        configs: vec![HeadlessConfig {
            file_paths: vec![dir.join("a.ts").to_string_lossy().into_owned()],
            rules: vec![
                HeadlessRule { name: "no-floating-promises".to_string(), options: None },
                HeadlessRule { name: "no-unnecessary-type-assertion".to_string(), options: None },
            ],
        }],
        source_overrides: None,
        report_syntactic: false,
        report_semantic: true,
    };
    let (tx, rx) = mpsc::channel();
    let sink: Sink = Box::new(move |out| tx.send(out).unwrap());
    spawn(payload, &dir.to_string_lossy(), opts.clone(), sink).unwrap().join().unwrap().unwrap();
    rx.into_iter()
        .map(|o| match o {
            Output::Diagnostic(d) => format!("{:?} {:?} {:?}", d.rule, d.range, d.message.id),
            other => panic!("unexpected output: {other:?}"),
        })
        .collect()
}

#[test]
fn repeated_runs_give_their_memory_back() {
    let dir = project();
    let start = reserved_in_use();
    let expected = diagnostics(&dir, &Options::default());
    let one_program = reserved_in_use() - start;
    assert_eq!(expected.len(), 1, "{expected:?}");
    let opts = Options { free_programs: true, ..Options::default() };
    for _ in 0..3 {
        assert_eq!(diagnostics(&dir, &opts), expected);
    }
    let before = reserved_in_use();
    for _ in 0..20 {
        assert_eq!(diagnostics(&dir, &opts), expected);
    }
    let grown = reserved_in_use().saturating_sub(before);
    assert!(
        grown < one_program,
        "20 runs kept {} KiB of the arena reservation, one kept program takes {} KiB",
        grown >> 10,
        one_program >> 10
    );
    std::fs::remove_dir_all(&dir).ok();
}
