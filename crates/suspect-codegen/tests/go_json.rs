//! Native consumers of the exact JSON runtime emitted for Go, plus the
//! public emission contract. The Go test vectors below are independent,
//! hand-authored fixtures: they are not mirrored from the tracked corpus and
//! they do not assert private emitter structure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use suspect_codegen::go_json;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

/// Writes the emitted module plus `extra` files into a deterministic scratch
/// directory. Fixtures are retained on failure for inspection.
fn go_directory(tag: &str, extra: &[(&str, &str)]) -> PathBuf {
    let directory = tempfile::Builder::new()
        .prefix(&format!("suspect-go-json-{tag}-"))
        .tempdir()
        .unwrap()
        .keep();
    for file in go_json::emit() {
        let path = directory.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let directory = directory.join("go");
    for (name, content) in extra {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    directory
}

/// Runs the native Go toolchain. A missing toolchain fails the opt-in gate;
/// it never silently skips.
fn run_go(directory: &Path, args: &[&str]) {
    let output = Command::new("go")
        .current_dir(directory)
        .args(args)
        .env("GOWORK", "off")
        .env(
            "GOTOOLCHAIN",
            std::env::var_os("SUSPECT_GO_TOOLCHAIN").unwrap_or_else(|| "local".into()),
        )
        .output()
        .expect("requires the native Go toolchain; the opt-in gate fails when it is missing");
    assert!(
        output.status.success(),
        "native go {} failed; fixtures retained in {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        args.join(" "),
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Go test vectors: exact numeric tokens, JSON grammar rejections, duplicate
/// decoded names, and the Unicode scalar domain.
const SDK_VECTORS_TEST: &str = r#"package sdk

import (
	"errors"
	"testing"
)

func expectKind(t *testing.T, err error, kind JSONErrorKind) {
	t.Helper()
	var jerr *JSONError
	if !errors.As(err, &jerr) {
		t.Fatalf("expected *JSONError, got %v", err)
	}
	if jerr.Kind != kind {
		t.Fatalf("expected kind %s, got %s (%s)", kind, jerr.Kind, jerr.Message)
	}
}

func TestExactNumberTokens(t *testing.T) {
	for token, integral := range map[string]bool{
		"1.0000000000000000001":    false,
		"9007199254740993":         true,
		"1e3":                      true,
		"1e-3":                     false,
		"-10.00e-1":                true,
		"-0":                       true,
		"1e999999":                 true,
		"1e-999999":                false,
		"0e-999999999999999999999": true,
		"1.5":                      false,
	} {
		number, err := ParseNumber(token)
		if err != nil {
			t.Fatalf("ParseNumber(%q): %v", token, err)
		}
		if number.String() != token {
			t.Fatalf("token changed: %q -> %q", token, number.String())
		}
		if number.IsInteger() != integral {
			t.Fatalf("IsInteger(%q) = %v", token, number.IsInteger())
		}
		if _, err := ParseInteger(token); (err == nil) != integral {
			t.Fatalf("ParseInteger(%q) acceptance mismatch: %v", token, err)
		}
	}
	for _, token := range []string{"", "01", "+1", "1.", ".1", "1e", "1e+", "NaN", "Infinity", "0x10", "1.2.3", "1 e3"} {
		if _, err := ParseNumber(token); err == nil {
			t.Fatalf("ParseNumber(%q) accepted", token)
		}
	}
	var zero Number
	var zeroInt Integer
	if zero.String() != "" || zero.IsInteger() {
		t.Fatal("zero Number must be invalid, never a substituted zero value")
	}
	if zeroInt.String() != "" {
		t.Fatal("zero Integer must carry no token")
	}
	if _, err := Encode(zero, DefaultLimits()); err == nil {
		t.Fatal("Encode must reject the zero Number")
	}
}

func TestExactRoundtrip(t *testing.T) {
	input := []byte(`{"a":[1e999999,-0,1.0000000000000000001],"b":{"x":"a\nb\\c"},"c":null,"d":9007199254740993}`)
	value, err := Parse(input, DefaultLimits())
	if err != nil {
		t.Fatal(err)
	}
	output, err := Encode(value, DefaultLimits())
	if err != nil {
		t.Fatal(err)
	}
	if string(output) != string(input) {
		t.Fatalf("roundtrip changed the document:\n in: %s\nout: %s", input, output)
	}
	again, err := Parse(output, DefaultLimits())
	if err != nil {
		t.Fatal(err)
	}
	stable, err := Encode(again, DefaultLimits())
	if err != nil {
		t.Fatal(err)
	}
	if string(stable) != string(output) {
		t.Fatal("unstable exact roundtrip")
	}
}

func TestParseRejections(t *testing.T) {
	for _, input := range []string{
		"", " ", "{", "[1,]", `{"a":1,}`, "true false", "{a:1}", `"\x61"`,
		"\"a\x00b\"", "01", "nul", `{"a":1}{"b":2}`, `{"a" 1}`, "true,true", "[",
	} {
		if _, err := Parse([]byte(input), DefaultLimits()); err == nil {
			t.Fatalf("Parse(%q) accepted", input)
		} else {
			expectKind(t, err, JSONSyntax)
		}
	}
	for _, input := range []string{
		`{"a":1,"a":2}`,
		`{"":1,"":2}`,
	} {
		if _, err := Parse([]byte(input), DefaultLimits()); err == nil {
			t.Fatalf("Parse(%q) accepted duplicate names", input)
		} else {
			expectKind(t, err, JSONDuplicateName)
		}
	}
}

func TestUnicodeDomain(t *testing.T) {
	if _, err := Parse([]byte("\"\xff\""), DefaultLimits()); err == nil {
		t.Fatal("raw invalid UTF-8 accepted")
	} else {
		expectKind(t, err, JSONBadUnicode)
	}
	for _, input := range []string{`"\uD800"`, `"\uDC00"`, `"\uDBFF"`, `"\uD800A"`} {
		if _, err := Parse([]byte(input), DefaultLimits()); err == nil {
			t.Fatalf("escaped lone surrogate accepted: %s", input)
		} else {
			expectKind(t, err, JSONBadUnicode)
		}
	}
	value, err := Parse([]byte(`"\uD834\uDD1E"`), DefaultLimits())
	if err != nil {
		t.Fatal(err)
	}
	if value != Value("\U0001D11E") {
		t.Fatalf("surrogate pair decoded wrong: %v", value)
	}
	for _, s := range []string{"\u00e9", "\uFFFE", "\uFFFD", "\"\\\b\f\n\r\t"} {
		output, err := Encode(s, DefaultLimits())
		if err != nil {
			t.Fatalf("Encode(%q): %v", s, err)
		}
		back, err := Parse(output, DefaultLimits())
		if err != nil {
			t.Fatalf("reparse %q: %v", output, err)
		}
		if back != Value(s) {
			t.Fatalf("string roundtrip changed %q into %v", s, back)
		}
	}
	if _, err := Encode("\xff", DefaultLimits()); err == nil {
		t.Fatal("Encode accepted a string that is not valid UTF-8")
	} else {
		expectKind(t, err, JSONBadUnicode)
	}
}
"#;

/// Go test vectors: per-call budgets, cycle detection, and encode domain
/// rejections.
const SDK_BUDGETS_TEST: &str = r#"package sdk

import (
	"strings"
	"testing"
)

func TestBudgets(t *testing.T) {
	limits := DefaultLimits()
	deep := "[" + strings.Repeat("[", 255) + "1" + strings.Repeat("]", 255) + "]"
	if _, err := Parse([]byte(deep), limits); err != nil {
		t.Fatal(err)
	}
	tooDeep := "[" + strings.Repeat("[", 256) + "1" + strings.Repeat("]", 256) + "]"
	if _, err := Parse([]byte(tooDeep), limits); err == nil {
		t.Fatal("depth 257 accepted with MaxDepth 256")
	} else {
		expectKind(t, err, JSONLimit)
	}
	few := DefaultLimits()
	few.MaxNodes = 3
	if _, err := Parse([]byte(`[1,[2,[3]]]`), few); err == nil {
		t.Fatal("value budget exceeded without error")
	} else {
		expectKind(t, err, JSONLimit)
	}
	scalar := DefaultLimits()
	scalar.MaxNodes = 1
	if _, err := Parse([]byte("null"), scalar); err != nil {
		t.Fatal(err)
	}
	tiny := DefaultLimits()
	tiny.MaxBytes = 3
	if _, err := Parse([]byte("true"), tiny); err == nil {
		t.Fatal("input byte budget exceeded without error")
	} else {
		expectKind(t, err, JSONLimit)
	}
	nl := DefaultLimits()
	nl.MaxNumberLength = 4
	if _, err := Parse([]byte("1e12345"), nl); err == nil {
		t.Fatal("numeric budget exceeded without error")
	} else {
		expectKind(t, err, JSONLimit)
	}
	integer, err := ParseInteger("9007199254740993")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Encode(integer, nl); err == nil {
		t.Fatal("numeric budget exceeded on encode without error")
	}
	bad := DefaultLimits()
	bad.MaxNodes = 0
	if _, err := Parse([]byte("null"), bad); err == nil {
		t.Fatal("nonpositive MaxNodes accepted")
	} else {
		expectKind(t, err, JSONLimit)
	}
	if _, err := Encode(nil, bad); err == nil {
		t.Fatal("nonpositive MaxNodes accepted on encode")
	} else {
		expectKind(t, err, JSONLimit)
	}
	smallOut := DefaultLimits()
	smallOut.MaxOutputBytes = 7
	if _, err := Encode("abcdef", smallOut); err == nil {
		t.Fatal("output byte budget exceeded without error")
	} else {
		expectKind(t, err, JSONLimit)
	}
}

func TestCycleDetection(t *testing.T) {
	object := map[string]Value{"next": nil}
	object["next"] = object
	if _, err := Encode(object, DefaultLimits()); err == nil {
		t.Fatal("cyclic object accepted")
	} else {
		expectKind(t, err, JSONCycle)
	}
	items := []Value{nil}
	items[0] = items
	if _, err := Encode(items, DefaultLimits()); err == nil {
		t.Fatal("cyclic slice accepted")
	} else {
		expectKind(t, err, JSONCycle)
	}
	shared := []Value{"x"}
	if _, err := Encode([]Value{shared, shared}, DefaultLimits()); err != nil {
		t.Fatal("repeated acyclic container rejected:", err)
	}
	empty := []Value{}
	if _, err := Encode([]Value{empty, empty}, DefaultLimits()); err != nil {
		t.Fatal("aliased empty container rejected:", err)
	}
}

func TestEncodeDomain(t *testing.T) {
	type point struct{ X int }
	for _, v := range []Value{
		1.5,
		float32(1.5),
		1,
		[]int{1, 2},
		map[int]string{1: "x"},
		point{X: 1},
		&point{X: 1},
		func() {},
		Number{},
		Integer{},
	} {
		if _, err := Encode(v, DefaultLimits()); err == nil {
			t.Fatalf("Encode accepted %T", v)
		} else {
			expectKind(t, err, JSONType)
		}
	}
}
"#;

#[test]
#[ignore = "requires the native Go toolchain"]
fn native_go_runtime_passes_exact_number_and_json_vectors() {
    let directory = go_directory("vectors", &[("json_test.go", SDK_VECTORS_TEST)]);
    run_go(&directory, &["test", "./..."]);
}

#[test]
#[ignore = "requires the native Go toolchain"]
fn native_go_runtime_rejects_over_budget_and_cyclic_values() {
    let directory = go_directory(
        "budgets",
        &[
            ("json_test.go", SDK_BUDGETS_TEST),
            ("vectors_test.go", SDK_VECTORS_TEST),
        ],
    );
    run_go(&directory, &["test", "./..."]);
}

#[test]
#[ignore = "requires the native Go toolchain"]
fn native_go_runtime_preserves_edge_values_and_exact_byte_limits() {
    let directory = go_directory(
        "edges",
        &[(
            "edge_test.go",
            r#"package sdk
import ("testing"; "strings"; "math/big"; "math/rand"; "fmt")
func TestExponentPadding(t *testing.T) {
    for _, token := range []string{"10e-0000000001", "1e-0000000000", "100e-0000000002"} {
        if _, err:=ParseInteger(token); err!=nil {t.Fatal(token,err)}
    }
    random:=rand.New(rand.NewSource(8259))
    for i:=0;i<10000;i++ {
        token:=fmt.Sprintf("%d.%06de%+d",random.Intn(1000000),random.Intn(1000000),random.Intn(81)-40)
        oracle,ok:=new(big.Rat).SetString(token);if !ok {t.Fatal(token)}
        n,err:=ParseNumber(token);if err!=nil || n.IsInteger()!=oracle.IsInt() {t.Fatal(token,err)}
    }
}
func TestNoUnicodeReplacementAfterPair(t *testing.T) {
    for _,text:=range []string{`["\ud83d\ude00","\ud800"]`,`["\uD83D\uDE00","\uDC00"]`} {
        if _,err:=Parse([]byte(text),DefaultLimits());err==nil {t.Fatal("surrogate replaced",text)}
    }
}
func TestExactOutputCeiling(t *testing.T) {
    for _,c:=range []struct{value Value;wire string}{{nil,"null"},{true,"true"},{"abcdef",`"abcdef"`},{[]Value{},"[]"},{map[string]Value{},"{}"}} {
        limits:=DefaultLimits();limits.MaxOutputBytes=len(c.wire)
        bytes,err:=Encode(c.value,limits);if err!=nil||string(bytes)!=c.wire {t.Fatal(c.wire,err,string(bytes))}
        limits.MaxOutputBytes--
        if _,err:=Encode(c.value,limits);err==nil {t.Fatal("output cap bypass",c.wire)}
    }
    limits:=DefaultLimits();limits.MaxDepth=257
    if _,err:=Parse([]byte("null"),limits);err==nil {t.Fatal("depth ceiling bypass")}
}
func TestFiniteAliasedSliceView(t *testing.T) {
    values:=make([]Value,2);values[1]=values[:1]
    encoded,err:=Encode(values,DefaultLimits());if err!=nil||string(encoded)!=`[null,[null]]` {t.Fatal(err,string(encoded))}
    _=strings.Builder{}
}
"#,
        )],
    );
    run_go(&directory, &["test", "./..."]);
}

/// Corpus consumer: parse, encode, reparse, reencode, and require a stable
/// exact roundtrip of the whole document.
const CORPUS_MAIN: &str = r#"package main

import (
	"os"

	"example.com/generated-json"
)

func main() {
	data, err := os.ReadFile("input.json")
	if err != nil {
		panic(err)
	}
	value, err := sdk.Parse(data, sdk.DefaultLimits())
	if err != nil {
		panic(err)
	}
	output, err := sdk.Encode(value, sdk.DefaultLimits())
	if err != nil {
		panic(err)
	}
	again, err := sdk.Parse(output, sdk.DefaultLimits())
	if err != nil {
		panic(err)
	}
	stable, err := sdk.Encode(again, sdk.DefaultLimits())
	if err != nil {
		panic(err)
	}
	if string(stable) != string(output) {
		panic("unstable exact JSON roundtrip")
	}
	if err := os.WriteFile("output.json", output, 0o644); err != nil {
		panic(err)
	}
}
"#;

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT, the native Go toolchain; missing inputs fail"]
fn tracked_openrouter_contract_documents_roundtrip_through_native_go_runtime() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .expect("set OPENROUTER_WEB_ROOT to the source checkout");
    for relative in [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ] {
        let path = Path::new(&root).join(relative);
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap())
            .expect("tracked OpenRouter contract input must load");
        let original = contract.document(contract.entry()).unwrap();
        let text = serde_json::to_string(original).unwrap();
        let directory = go_directory("corpus", &[("cmd/main.go", CORPUS_MAIN)]);
        std::fs::write(directory.join("input.json"), &text).unwrap();
        run_go(&directory, &["run", "./cmd"]);
        let encoded = std::fs::read(directory.join("output.json")).unwrap();
        let roundtripped: serde_json::Value = serde_json::from_slice(&encoded)
            .unwrap_or_else(|error| panic!("{relative}: runtime produced invalid JSON: {error}"));
        assert_eq!(
            &roundtripped, original,
            "{relative}: the Go runtime changed the contract document"
        );
    }
}
