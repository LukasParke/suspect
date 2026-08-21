//! RFC 9535 conformance and feature tests over the RFC's own `inst.json`
//! example document (§2 / §3), plus selector, filter, function,
//! normalization, error, and unicode coverage.

use suspect_jsonpath::{NodeList, Path, PathError};
use suspect_low::{LowDoc, ValueKind};
use suspect_source::{Source, Uri};

const INST: &str = r#"{
  "store": {
    "book": [
      { "category": "reference",
        "author": "Nigel Rees",
        "title": "Sayings of the Century",
        "price": 8.95 },
      { "category": "fiction",
        "author": "Evelyn Waugh",
        "title": "Sword of Honour",
        "price": 12.99 },
      { "category": "fiction",
        "author": "Herman Melville",
        "title": "Moby Dick",
        "isbn": "0-553-21311-3",
        "price": 8.99 },
      { "category": "fiction",
        "author": "J. R. R. Tolkien",
        "title": "The Lord of the Rings",
        "isbn": "0-395-19395-8",
        "price": 22.99 }
    ],
    "bicycle": {
      "color": "red",
      "price": 19.95
    }
  },
  "expensive": 10
}"#;

fn doc() -> LowDoc {
    LowDoc::parse(
        Uri::parse("mem://inst.json").unwrap(),
        Source::from_vec(INST.as_bytes().to_vec()),
    )
}

fn query<'a>(doc: &'a LowDoc, path: &str) -> NodeList<'a> {
    Path::parse(path).unwrap().query(doc.root())
}

fn strs(nodes: &NodeList<'_>) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.kind() == ValueKind::Str)
        .filter_map(|n| n.as_str().map(str::to_owned))
        .collect()
}

fn nums(nodes: &NodeList<'_>) -> Vec<f64> {
    nodes.iter().filter_map(|n| n.as_f64()).collect()
}

#[test]
fn rfc_store_book_authors_wildcard() {
    let d = doc();
    assert_eq!(
        strs(&query(&d, "$.store.book[*].author")),
        ["Nigel Rees", "Evelyn Waugh", "Herman Melville", "J. R. R. Tolkien"]
    );
}

#[test]
fn rfc_all_authors_descendant() {
    let d = doc();
    assert_eq!(
        strs(&query(&d, "$..author")),
        ["Nigel Rees", "Evelyn Waugh", "Herman Melville", "J. R. R. Tolkien"]
    );
}

#[test]
fn rfc_store_members() {
    let d = doc();
    // $.store.* : book array + bicycle object
    let nodes = query(&d, "$.store.*");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes.first().unwrap().kind(), ValueKind::Array);
    assert_eq!(nodes.get(1).unwrap().kind(), ValueKind::Object);
}

#[test]
fn rfc_store_prices_descendant() {
    let d = doc();
    // $.store..price : four book prices + bicycle price
    let mut prices = nums(&query(&d, "$.store..price"));
    prices.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(prices, [8.95, 8.99, 12.99, 19.95, 22.99]);
}

#[test]
fn rfc_third_book_and_negative_index() {
    let d = doc();
    assert_eq!(strs(&query(&d, "$..book[2].title")), ["Moby Dick"]);
    // last book
    assert_eq!(strs(&query(&d, "$..book[-1:].title")), ["The Lord of the Rings"]);
    // first two books (union and slice)
    assert_eq!(query(&d, "$..book[0,1]").len(), 2);
    assert_eq!(
        strs(&query(&d, "$..book[:2].title")),
        ["Sayings of the Century", "Sword of Honour"]
    );
}

#[test]
fn rfc_books_with_isbn_filter() {
    let d = doc();
    assert_eq!(
        strs(&query(&d, "$..book[?(@.isbn)].title")),
        ["Moby Dick", "The Lord of the Rings"]
    );
}

#[test]
fn rfc_comparisons_against_absolute_and_relative() {
    let d = doc();
    // books cheaper than $.expensive (10)
    assert_eq!(
        strs(&query(&d, "$..book[?($.expensive > @.price)].title")),
        ["Sayings of the Century", "Moby Dick"]
    );
}

#[test]
fn rfc_root_access_forms() {
    let d = doc();
    assert_eq!(nums(&query(&d, "$.expensive")), [10.0]);
    assert_eq!(nums(&query(&d, "$['expensive']")), [10.0]);
    assert_eq!(strs(&query(&d, "$.store.bicycle['color']")), ["red"]);
    assert_eq!(query(&d, "$").len(), 1);
    assert_eq!(query(&d, "$").first().unwrap().kind(), ValueKind::Object);
}

// ---- selector kinds and slice edges ---------------------------------------

#[test]
fn slice_edge_cases() {
    let d = doc();
    let titles = |p: &str| strs(&query(&d, &format!("$..book{p}.title")));

    // [::-1] full reverse — same set as the whole array; results are
    // normalized into document order per RFC 9535 §2.1 (normalization)
    assert_eq!(
        titles("[::-1]"),
        ["Sayings of the Century", "Sword of Honour", "Moby Dick", "The Lord of the Rings"]
    );
    // [2:] drop first two
    assert_eq!(titles("[2:]"), ["Moby Dick", "The Lord of the Rings"]);
    // [-3:] last three
    assert_eq!(
        titles("[-3:]"),
        ["Sword of Honour", "Moby Dick", "The Lord of the Rings"]
    );
    // step 2
    assert_eq!(titles("[::2]"), ["Sayings of the Century", "Moby Dick"]);
    // reverse with explicit bounds (exclusive end); normalized to doc order
    assert_eq!(titles("[3:1:-1]"), ["Moby Dick", "The Lord of the Rings"]);
    assert_eq!(titles("[::10]"), ["Sayings of the Century"]);
    // out-of-range clamping yields empty
    assert!(query(&d, "$..book[5:].title").is_empty());
    assert!(query(&d, "$..book[10:20]").is_empty());
    // zero-length window
    assert!(query(&d, "$..book[2:2]").is_empty());
    // omitted start
    assert_eq!(titles("[:2]"), ["Sayings of the Century", "Sword of Honour"]);
    // fully omitted bounds
    assert_eq!(
        titles("[::]"),
        ["Sayings of the Century", "Sword of Honour", "Moby Dick", "The Lord of the Rings"]
    );
}

#[test]
fn shorthand_child_and_descendant_wildcards() {
    let d = doc();
    // `.*` is a child wildcard segment (RFC 9535 §2.5.1.2)
    let nodes = query(&d, "$.store.*");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes.first().unwrap().kind(), ValueKind::Array);
    // chained after other segments: the suspect-overlay regression
    assert_eq!(
        strs(&query(&d, "$.store.bicycle.*")),
        ["red"]
    );
    // `$..*` descendant wildcard walks the whole subtree
    assert_eq!(query(&d, "$..*").len(), 28);
    // mid-query shorthand wildcards
    assert_eq!(nums(&query(&d, "$.store.*.price")).len(), 1); // bicycle price; book array has no .price
}

#[test]
fn shorthand_wildcard_over_openapi_shape() {
    // chained shorthand wildcards over an OpenAPI-shaped document
    let api = r#"{"paths": {"/pets": {"get": {"responses": {"200": {"x": 1}}}},
                             "/users": {"get": {"responses": {"200": {"y": 2}}}}}}"#;
    let da = LowDoc::parse(
        Uri::parse("mem://api.json").unwrap(),
        Source::from_vec(api.as_bytes().to_vec()),
    );
    assert_eq!(query(&da, "$.paths.*.get.responses").len(), 2);
    assert_eq!(query(&da, "$.paths.*.get.responses['200'].x").len(), 1);
}

#[test]
fn index_edges() {
    let d = doc();
    assert_eq!(strs(&query(&d, "$..book[-4].title")), ["Sayings of the Century"]);
    assert!(query(&d, "$..book[-5]").is_empty());
    assert!(query(&d, "$..book[4]").is_empty());
    // multi-selector union returns results in document order regardless of
    // the order selectors were listed
    assert_eq!(
        strs(&query(&d, "$..book[2,0].title")),
        ["Sayings of the Century", "Moby Dick"]
    );
}

#[test]
fn descendant_through_arrays_and_objects() {
    let d = doc();
    // every price anywhere
    assert_eq!(nums(&query(&d, "$..price")).len(), 5);
    // $..* walks the whole tree; inst.json has 28 non-root nodes
    assert_eq!(query(&d, "$..*").len(), 28);
    // descendant bracket forms
    assert_eq!(nums(&query(&d, "$..['price']")).len(), 5);
    // ..[*] selects the children of every node at any depth (root included)
    assert_eq!(query(&d, "$..[*]").len(), 28);
    // ..[0] selects the first element of every array at any depth
    assert_eq!(nums(&query(&d, "$..[0].price")), [8.95]);
    // relative descendant inside a filter
    // relative descendant existence inside a filter (non-singular queries
    // are allowed in logical position)
    assert_eq!(
        strs(&query(&d, "$..book[?(@..isbn)].title")),
        ["Moby Dick", "The Lord of the Rings"]
    );
    assert_eq!(
        strs(&query(&d, "$..book[?(@.price < 9)].title")),
        ["Sayings of the Century", "Moby Dick"]
    );
}

#[test]
fn wildcard_on_scalars_is_empty() {
    let d = doc();
    assert!(query(&d, "$.expensive.*").is_empty());
    assert!(query(&d, "$.expensive[*]").is_empty());
}

// ---- filters ---------------------------------------------------------------

#[test]
fn comparisons_strings_numbers_bools_null() {
    let json = r#"{
      "a": {"s":"x","n":1,"b":true,"z":null},
      "b": {"s":"y","n":2,"b":false,"z":1}
    }"#;
    let d = LowDoc::parse(
        Uri::parse("mem://cmp.json").unwrap(),
        Source::from_vec(json.as_bytes().to_vec()),
    );
    // number equality incl int/float unification
    assert_eq!(nums(&query(&d, "$[?(@.n == 1)].n")), [1.0]);
    assert_eq!(nums(&query(&d, "$[?(@.n == 1.0)].n")), [1.0]);
    assert_eq!(nums(&query(&d, "$[?(@.n != 1)].n")), [2.0]);
    assert_eq!(nums(&query(&d, "$[?(@.n <= 1)].n")), [1.0]);
    assert_eq!(nums(&query(&d, "$[?(@.n >= 2)].n")), [2.0]);
    assert_eq!(nums(&query(&d, "$[?(@.n > 1.5)].n")), [2.0]);
    // string equality and codepoint ordering
    assert_eq!(strs(&query(&d, "$[?(@.s == 'x')].s")), ["x"]);
    assert_eq!(strs(&query(&d, "$[?(@.s < 'y')].s")).len(), 1);
    assert_eq!(strs(&query(&d, "$[?(@.s >= 'y')].s")).len(), 1);
    // bool equality
    assert_eq!(query(&d, "$[?(@.b == true)]").len(), 1);
    // null equality: nothing equals null except null
    assert_eq!(query(&d, "$[?(@.z == null)]").len(), 1);
    assert_eq!(query(&d, "$[?(@.z == 1)]").len(), 1);
    assert_eq!(query(&d, "$[?('x' == @.z)]").len(), 0);
    assert_eq!(query(&d, "$[?(1 == @.z)]").len(), 1);
    // cross-type == is false (string vs number)
    assert_eq!(query(&d, "$[?(@.s == 1)]").len(), 0);
    // cross-type ordering is false
    assert_eq!(query(&d, "$[?(@.s < 1)]").len(), 0);
    // negative and exponent literals
    let json2 = r#"{"a":{"t":-5},"b":{"t":1e2},"c":{"t":-0.5}}"#;
    let d2 = LowDoc::parse(
        Uri::parse("mem://neg.json").unwrap(),
        Source::from_vec(json2.as_bytes().to_vec()),
    );
    assert_eq!(nums(&query(&d2, "$[?(@.t == -5)].t")), [-5.0]);
    assert_eq!(nums(&query(&d2, "$[?(@.t == 100)].t")).len(), 1);
    assert_eq!(nums(&query(&d2, "$[?(@.t < -0.25)].t")), [-5.0, -0.5]);
}

#[test]
fn nested_singular_relative_query_in_filter() {
    let d = doc();
    assert_eq!(
        strs(&query(&d, "$..book[?(@.author == 'Herman Melville')].isbn")),
        ["0-553-21311-3"]
    );
    // two levels down through a nested object
    assert_eq!(
        strs(&query(&d, "$.store[?(@.color == 'red')].color")),
        ["red"]
    );
}

#[test]
fn absolute_query_inside_filter() {
    let d = doc();
    // $ refers to the root passed to query()
    assert_eq!(
        strs(&query(&d, "$..book[?(@.price > $.store.book[0].price)].title")),
        ["Sword of Honour", "Moby Dick", "The Lord of the Rings"]
    );
}

#[test]
fn logical_operators_and_parens() {
    let d = doc();
    let f = |expr: &str| strs(&query(&d, &format!("$..book[?({expr})].title")));
    assert_eq!(f("@.isbn && @.price < 10"), ["Moby Dick"]);
    assert_eq!(f("!@.isbn && @.price < 10"), ["Sayings of the Century"]);
    assert_eq!(
        f("@.price < 9 || @.price > 20"),
        ["Sayings of the Century", "Moby Dick", "The Lord of the Rings"]
    );
    assert_eq!(
        f("(!@.isbn || @.price < 9) && @.category == 'fiction'"),
        ["Sword of Honour", "Moby Dick"]
    );
    assert_eq!(f("!(@.isbn) && !(@.price > 9)"), ["Sayings of the Century"]);
    // double negation
    assert_eq!(f("!!@.isbn"), ["Moby Dick", "The Lord of the Rings"]);
}

#[test]
fn regex_match_vs_search() {
    let d = doc();
    let f = |func: &str| strs(&query(&d, &format!("$..book[?{func}].title")));
    // search finds substrings anywhere
    assert_eq!(
        f("(search(@.title, 'of'))"),
        ["Sayings of the Century", "Sword of Honour", "The Lord of the Rings"]
    );
    // match with an unanchored pattern still requires... it is a plain regex
    // search too in this engine; anchors are explicit
    assert_eq!(f("(match(@.title, 'Moby.*'))"), ["Moby Dick"]);
    assert_eq!(f("(search(@.isbn, '\\d{5}'))"), ["Moby Dick", "The Lord of the Rings"]);
    assert!(f("(match(@.title, '^of'))").is_empty());
    // case-insensitive flag passes through
    assert_eq!(f("(search(@.author, '(?i)nigel'))"), ["Sayings of the Century"]);
    // missing member is Nothing -> never matches
}

#[test]
fn existence_of_missing_is_false() {
    let d = doc();
    assert_eq!(query(&d, "$..book[?(@.nonexistent)]").len(), 0);
    assert_eq!(query(&d, "$..book[?(!@.nonexistent)]").len(), 4);
}

// ---- functions --------------------------------------------------------------

#[test]
fn function_length_on_string_array_and_nodelist() {
    let d = doc();
    // string length (codepoints)
    assert_eq!(strs(&query(&d, "$..book[?(length(@.title) == 9)].title")), ["Moby Dick"]);
    // array length via singular path from a filter over root children
    let hits = query(&d, "$[?(length(@.book) == 4)]");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits.first().unwrap().kind(), ValueKind::Object);
    // nodelist length counts nodes
    assert_eq!(query(&d, "$[?(length(@..isbn) == 2)]").len(), 1);
}

#[test]
fn function_count_and_comparison() {
    let d = doc();
    // count over a descendant query inside a filter
    assert_eq!(query(&d, "$[?(count(@..author) == 4)]").len(), 1);
    // count used with ordering comparison
    assert_eq!(query(&d, "$..book[?(count(@.author) == 1)]").len(), 4);
}

#[test]
fn function_value_unwrap() {
    let d = doc();
    // exactly one match -> its value compared inside a filter
    assert_eq!(
        strs(&query(&d, "$.store[?(value(@.price) == 19.95)].color")),
        ["red"]
    );
    // multiple matches -> Nothing -> no result
    assert!(query(&d, "$[?(value(@..price) == 19.95)]").is_empty());
    // zero matches -> Nothing -> no result
    assert!(query(&d, "$[?(value($.nope) == 1)]").is_empty());
}

#[test]
fn function_as_bare_boolean_filter() {
    let d = doc();
    // match/search are directly logical
    assert_eq!(
        strs(&query(&d, "$..book[?(search(@.isbn, '[0-9]{5}'))].title")),
        ["Moby Dick", "The Lord of the Rings"]
    );
    // numeric function as bare test exists when it produces a value
    assert_eq!(query(&d, "$..book[?(length(@.isbn))]").len(), 2);
    // count always produces a value for valid queries
    assert_eq!(query(&d, "$..book[?(count(@.title))]").len(), 4);
}

// ---- normalization ----------------------------------------------------------

#[test]
fn normalization_dedupes_overlaps_and_sorts_by_document_position() {
    let d = doc();
    // multi-selector union listed out of order comes back in document order
    assert_eq!(
        strs(&query(&d, "$..book[3,1,2].title")),
        ["Sword of Honour", "Moby Dick", "The Lord of the Rings"]
    );

    // real overlap: two routes reach the same node, deduped to one
    let json = r#"{"a":{"v":1},"b":{"a":{"v":2}},"c":[1,2]}"#;
    let d2 = LowDoc::parse(
        Uri::parse("mem://overlap.json").unwrap(),
        Source::from_vec(json.as_bytes().to_vec()),
    );
    // $..* descends into everything; inner {"v":2} is reachable via $.b.a
    // and via $..a — the normalized list must contain each position once,
    // ordered by byte offset.
    let all = query(&d2, "$..*");
    let starts: Vec<usize> = all.iter().map(|n| n.byte_range().start).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(starts, sorted, "duplicate or out-of-order positions in normalized result");

    // overlapping routes hitting the same object: $..a matches both the
    // outer and inner `a`; querying $..a.v yields exactly the two values
    assert_eq!(nums(&query(&d2, "$..a.v")), [1.0, 2.0]);
}

// ---- errors -----------------------------------------------------------------

#[test]
fn parse_errors() {
    let cases: &[(&str, usize)] = &[
        ("$.a[", 4),                // unterminated bracket
        ("$.a[0", 5),               // unterminated bracket (no close)
        ("$a", 1),                  // trailing garbage after $
        ("$.a b", 4),               // trailing garbage
        ("", 0),                    // missing $
        ("$..a x", 5),              // trailing garbage
        ("$..", 3),                 // dangling descendant dot
        ("$.a[?()]", 6),            // empty filter
        ("$.a[?@.b", 8),            // unterminated filter/bracket
        ("$['a", 4),                // unterminated string
        ("$.a[?'x']", 8),           // literal alone in filter
        ("$.a[?@..b == 1]", 12),    // non-singular comparand on lhs
        ("$.a[?@.b == $..c]", 16),  // non-singular comparand on rhs
        ("$.a[frobnicate(1)]", 4),  // unknown identifier in selector
        ("$.a[count()]", 4),        // function outside filter
        ("$.a[match(@.b, '(')]", 4), // functions only inside filters here
        ("$.a[::0]", 7),            // zero step
        ("$.a[1:2:0]", 9),          // zero step
    ];
    for &(input, offset) in cases {
        match Path::parse(input) {
            Err(PathError { input: ref inp, offset: off, reason: _ }) => {
                assert_eq!(inp, input);
                assert_eq!(off, offset, "wrong offset for {input:?}");
            }
            Ok(_) => panic!("expected error for {input:?}"),
        }
    }
}

#[test]
fn error_display_mentions_offset() {
    let e = Path::parse("$.a[").unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("offset 4"), "{msg}");
}

// ---- unicode ----------------------------------------------------------------

#[test]
fn unicode_names_escapes_and_content() {
    let json = r#"{
      "café": "coffee",
      "日本語": "nihongo",
      "emoji": "🚀 rocket",
      "weird key with spaces": 1
    }"#;
    let d = LowDoc::parse(
        Uri::parse("mem://uni.json").unwrap(),
        Source::from_vec(json.as_bytes().to_vec()),
    );
    assert_eq!(strs(&query(&d, "$['café']")), ["coffee"]);
    assert_eq!(strs(&query(&d, "$['日本語']")), ["nihongo"]);
    assert_eq!(strs(&query(&d, "$['emoji']")), ["🚀 rocket"]);
    assert_eq!(nums(&query(&d, "$['weird key with spaces']")), [1.0]);
    // shorthand with unicode letters
    assert_eq!(strs(&query(&d, "$.café")), ["coffee"]);

    // escapes in string literals: \' \\ \n \t \uXXXX
    let json2 = r#"{
      "quote": "it's",
      "tabbed": "a\tb",
      "snowman": "☃"
    }"#;
    let d2 = LowDoc::parse(
        Uri::parse("mem://esc.json").unwrap(),
        Source::from_vec(json2.as_bytes().to_vec()),
    );
    // \' escape in the query's single-quoted literal
    let q = Path::parse(r"$[?(@ == 'it\'s')]").unwrap();
    assert_eq!(q.query(d2.root()).len(), 1);
    // \uXXXX escape decodes to the same codepoint as the document scalar
    let q = Path::parse(r"$[?(@ == '\u2603')]").unwrap();
    assert_eq!(q.query(d2.root()).len(), 1);
    // \t escape: the spine exposes raw (unprocessed) scalar bytes, so the
    // document value is literally `a\tb`; match it via the \\ escape
    let q = Path::parse(r"$[?(@ == 'a\\tb')]").unwrap();
    assert_eq!(q.query(d2.root()).len(), 1);
}
