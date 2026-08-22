//! Pack-level integration tests: every builtin rule has a firing and a clean
//! fixture; plus determinism and corpus parity spot-checks.

use suspect_lint::{Finding, Linter, Severity};
use suspect_low::{LowDoc, SpecFamily};
use suspect_source::{Source, Uri};

fn doc(yaml: &str) -> LowDoc {
    LowDoc::parse(
        Uri::parse("memory://fixture.yaml").expect("static uri"),
        Source::from_vec(yaml.as_bytes().to_vec()),
    )
}

fn run<'d>(linter: &Linter, doc: &'d LowDoc) -> Vec<Finding<'d>> {
    linter.run(doc)
}

fn codes<'a>(findings: &'a [Finding<'a>]) -> Vec<&'a str> {
    findings.iter().map(|f| &*f.code).collect()
}

/// One fixture pair per builtin rule.
struct Case {
    code: &'static str,
    firing: &'static str,
    clean: &'static str,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            code: "oas3-api-servers",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\nservers: null\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\nservers:\n  - url: https://example.com\npaths: {}\n",
        },
        Case {
            code: "oas3-api-contact",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\", contact: {name: n}}\npaths: {}\n",
        },
        Case {
            code: "info-contact",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\", contact: {email: a@b.c}}\npaths: {}\n",
        },
        Case {
            code: "info-license",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\", license: {name: MIT}}\npaths: {}\n",
        },
        Case {
            code: "license-url",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\", license: {name: MIT}}\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\", license: {name: MIT, url: https://x}}\npaths: {}\n",
        },
        Case {
            code: "openapi-tags",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ntags: null\npaths: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ntags:\n  - name: pets\npaths: {}\n",
        },
        Case {
            code: "operation-tags",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      tags: [pets]\n",
        },
        Case {
            code: "operation-operationId",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      operationId: getA\n",
        },
        Case {
            code: "operation-summary",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      summary: \"\"\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      summary: gets a\n",
        },
        Case {
            code: "operation-description",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      description: \"\"\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      description: does things\n",
        },
        Case {
            code: "operation-default-response",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      responses:\n        '404':\n          description: nope\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      responses:\n        default:\n          description: err\n",
        },
        Case {
            code: "operation-success-response",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      responses:\n        default:\n          description: err\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get:\n      responses:\n        '204':\n          description: ok\n",
        },
        Case {
            code: "path-params",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets/{petId}:\n    get:\n      operationId: x\n      responses:\n        '200': {description: ok}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets/{petId}:\n    parameters:\n      - name: petId\n        in: path\n        required: true\n        schema: {type: string}\n    get:\n      operationId: x\n      responses:\n        '200': {description: ok}\n",
        },
        Case {
            code: "path-keys-no-trailing-slash",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets/:\n    get: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets:\n    get: {}\n",
        },
        Case {
            code: "no-$ref-siblings",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    Pet:\n      $ref: '#/components/schemas/Other'\n      title: extra\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    Pet:\n      $ref: '#/components/schemas/Other'\n",
        },
        Case {
            code: "typed-enum",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    E:\n      type: string\n      enum: [one, 2, three]\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    E:\n      type: string\n      enum: [one, two]\n",
        },
        Case {
            code: "no-ambiguous-paths",
            firing: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets/{id}:\n    get: {}\n  /pets/{name}:\n    get: {}\n",
            clean: "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /pets:\n    get: {}\n  /pets/{id}:\n    get: {}\n",
        },
        Case {
            code: "overlay-info-description",
            firing: "overlay: \"1.0.0\"\ninfo: {title: t}\nactions: []\n",
            clean: "overlay: \"1.0.0\"\ninfo: {title: t, description: d}\nactions: []\n",
        },
        Case {
            code: "overlay-action-description",
            firing: "overlay: \"1.0.0\"\ninfo: {title: t, description: d}\nactions:\n  - target: $.info\n    update: {}\n",
            clean: "overlay: \"1.0.0\"\ninfo: {title: t, description: d}\nactions:\n  - target: $.info\n    description: adds stuff\n    update: {}\n",
        },
        Case {
            code: "arazzo-workflow-description",
            firing: "arazzo: \"1.0.0\"\ninfo: {title: t}\nsourceDefinitions: []\nworkflows:\n  - workflowId: w\n    steps: []\n",
            clean: "arazzo: \"1.0.0\"\ninfo: {title: t}\nsourceDefinitions: []\nworkflows:\n  - workflowId: w\n    description: does things\n    steps: []\n",
        },
        Case {
            code: "arazzo-step-operation",
            firing: "arazzo: \"1.0.0\"\ninfo: {title: t}\nsourceDefinitions: []\nworkflows:\n  - workflowId: w\n    description: d\n    steps:\n      - stepId: s1\n      - stepId: s2\n        operationId: op_a\n        operationPath: '{$sourceDescriptions.a#/paths/~1pets/get}'\n",
            clean: "arazzo: \"1.0.0\"\ninfo: {title: t}\nsourceDefinitions: []\nworkflows:\n  - workflowId: w\n    description: d\n    steps:\n      - stepId: s1\n        operationId: op_a\n",
        },
    ]
}

#[test]
fn every_pack_rule_fires_and_cleans() {
    let linter = Linter::spectral_default();
    for case in cases() {
        let firing_doc = doc(case.firing);
        let firing_hits = run(&linter, &firing_doc);
        assert!(
            codes(&firing_hits).contains(&case.code),
            "`{}` must fire on its firing fixture; got {:?}",
            case.code,
            codes(&firing_hits)
        );
        let clean_doc = doc(case.clean);
        let clean_hits = run(&linter, &clean_doc);
        assert!(
            !codes(&clean_hits).contains(&case.code),
            "`{}` must not fire on its clean fixture; got {:?}",
            case.code,
            codes(&clean_hits)
        );
    }
}

#[test]
fn ref_siblings_severity_tracks_oas_version() {
    let linter = Linter::spectral_default();
    let v30 = doc(
        "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    Pet:\n      $ref: '#/x'\n      title: extra\n",
    );
    let hits = run(&linter, &v30);
    let hit = hits
        .iter()
        .find(|f| &*f.code == "no-$ref-siblings")
        .expect("fires on 3.0");
    assert_eq!(hit.severity, Severity::Error);
    let v31 = doc(
        "openapi: \"3.1.0\"\ninfo: {title: t, version: \"1\"}\ncomponents:\n  schemas:\n    Pet:\n      $ref: '#/x'\n      title: extra\n",
    );
    let hits = run(&linter, &v31);
    let hit = hits
        .iter()
        .find(|f| &*f.code == "no-$ref-siblings")
        .expect("fires on 3.1");
    assert_eq!(hit.severity, Severity::Warn);
}

#[test]
fn findings_are_deterministic_and_sorted() {
    let messy = "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\ntags: null\npaths:\n  /b/:\n    get: {}\n  /a/:\n    post:\n      summary: \"\"\n";
    let d1 = doc(messy);
    let d2 = doc(messy);
    let first = run(&Linter::spectral_default(), &d1);
    let second = run(&Linter::spectral_default(), &d2);
    assert!(!first.is_empty());
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.code, b.code);
        assert_eq!(a.range, b.range);
        assert_eq!(a.path.to_path(), b.path.to_path());
    }
    for w in first.windows(2) {
        let key = |f: &Finding<'_>| (f.range.start, f.range.end, f.code.clone());
        assert!(
            key(&w[0]) <= key(&w[1]),
            "findings not sorted: {:?}",
            codes(&first)
        );
    }
}

#[test]
fn oas2_docs_only_get_shared_rules() {
    let d = doc("swagger: \"2.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get: {}\n");
    let hits = run(&Linter::spectral_default(), &d);
    assert!(!codes(&hits).contains(&"oas3-api-servers"));
    assert!(!codes(&hits).contains(&"oas3-api-contact"));
    assert!(codes(&hits).contains(&"operation-operationId"));
}

#[test]
fn unknown_family_documents_produce_no_builtin_findings() {
    let d = doc("random: doc\nnot: an-oas-spec\n");
    let hits = run(&Linter::spectral_default(), &d);
    assert!(hits.is_empty(), "{:?}", codes(&hits));
}

#[test]
fn petstore_corpus_lints_cleanly_with_real_ranges() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/petstore-expanded.yaml"
    );
    let bytes = std::fs::read(path).expect("corpus file readable");
    let source_len = bytes.len();
    let uri = Uri::from_path(std::path::Path::new(path)).expect("uri");
    let doc = LowDoc::parse(uri, Source::from_vec(bytes));
    assert_eq!(doc.sniff_family(), SpecFamily::Oas30);
    let findings = Linter::spectral_default().run(&doc);
    assert!(
        !findings.is_empty(),
        "expected at least one finding on petstore"
    );
    for f in &findings {
        assert!(
            f.range.start < f.range.end && f.range.end <= source_len,
            "bad range {:?}",
            f.range
        );
    }
    // Every petstore operation carries an operationId, so that rule stays
    // quiet while structural rules fire.
    assert!(!codes(&findings).contains(&"operation-operationId"));
    assert!(
        codes(&findings).contains(&"operation-tags"),
        "petstore operations have no tags; got {:?}",
        codes(&findings)
    );
    // Finding pointers resolve to real nodes in the document.
    let root = doc.root();
    for f in &findings {
        if f.path.is_root() {
            continue;
        }
        assert!(
            root.pointer(&f.path).is_some(),
            "finding path {} does not resolve",
            f.path.to_path()
        );
    }
}
