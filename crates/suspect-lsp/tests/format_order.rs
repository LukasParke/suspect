//! Canonical reordering: comment preservation, table order, extension
//! anchors, special sorts, idempotence.

use suspect_lsp::extensions_config::ExtensionConfig;
use suspect_lsp::format_order::canonical_format;

fn fmt(text: &str) -> String {
    canonical_format(text, true, &ExtensionConfig::default())
}

#[test]
fn root_keys_sort_canonically() {
    let text = "\
paths: {}
info:
  title: T
  version: '1'
openapi: 3.1.0
";
    let out = fmt(text);
    let order: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with(' ') && !l.is_empty())
        .filter_map(|l| l.split(':').next())
        .collect();
    assert_eq!(order, ["openapi", "info", "paths"], "{out}");
}

#[test]
fn every_comment_survives_reordering() {
    let text = "\
# top of file
openapi: 3.1.0
# info lives here
info: # inline note
  title: T
  version: '1'
paths: {}
# bottom note
";
    let out = fmt(text);
    for comment in [
        "# top of file",
        "# info lives here",
        "# inline note",
        "# bottom note",
    ] {
        assert!(out.contains(comment), "lost `{comment}` in:\n{out}");
    }
    // The info comment travels with info, above it.
    let info = out.find("\ninfo:").unwrap_or(out.find("info:").unwrap());
    let comment = out.find("# info lives here").unwrap();
    assert!(comment < info, "comment must precede its key:\n{out}");
    // Inline note stays on the info line.
    let info_line = out.lines().find(|l| l.starts_with("info:")).unwrap();
    assert!(info_line.contains("# inline note"), "{out}");
}

#[test]
fn standalone_comments_stay_put() {
    let text = "\
openapi: 3.1.0
info:
  title: T
  version: '1'

# section boundary comment

paths: {}
";
    let out = fmt(text);
    assert!(out.contains("# section boundary comment"), "{out}");
    // It stays between info and paths (blank-line separated groups do not
    // travel with the previous pair).
    let info_end = out.find("version: '1'").unwrap();
    let comment = out.find("# section boundary comment").unwrap();
    let paths = out.find("paths:").unwrap();
    assert!(info_end < comment && comment < paths, "{out}");
}

#[test]
fn operation_keys_sort_per_table() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /p:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
      description: does things
      parameters:
        - name: q
          in: query
          schema: {type: string}
      summary: Short
      operationId: doThing
";
    let out = fmt(text);
    // Absolute positions: operationId sorts first, then summary,
    // description, parameters — per the operation table.
    let operation_id = out.find("operationId:").unwrap();
    let summary = out.find("summary:").unwrap();
    let description = out.find("description: does").unwrap();
    let parameters = out.find("parameters:").unwrap();
    assert!(operation_id < summary, "{out}");
    assert!(summary < description, "{out}");
    assert!(description < parameters, "{out}");
}

#[test]
fn schema_keys_group_constraints() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
components:
  schemas:
    Pet:
      required: [name]
      properties:
        name: {type: string}
      type: object
      description: A pet
      maximum: 10
      minimum: 1
";
    let out = fmt(text);
    let pet = &out[out.find("Pet:").unwrap()..];
    let description = pet.find("description:").unwrap();
    let pet_type = pet.find("type: object").unwrap();
    let minimum = pet.find("minimum:").unwrap();
    let maximum = pet.find("maximum:").unwrap();
    let properties = pet.find("properties:").unwrap();
    let required = pet.find("required:").unwrap();
    assert!(description < pet_type, "{out}");
    assert!(minimum < maximum, "{out}");
    assert!(pet_type < minimum, "{out}");
    assert!(properties < required, "{out}");
}

#[test]
fn response_codes_sort_numerically() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /p:
    get:
      operationId: getP
      responses:
        '500': {description: e, content: {application/json: {schema: {type: object}}}}
        '200': {description: ok, content: {application/json: {schema: {type: object}}}}
        default: {description: d, content: {application/json: {schema: {type: object}}}}
        '404': {description: n, content: {application/json: {schema: {type: object}}}}
";
    let out = fmt(text);
    let responses = &out[out.find("responses:").unwrap()..];
    let c200 = responses.find("'200':").unwrap();
    let c404 = responses.find("'404':").unwrap();
    let c500 = responses.find("'500':").unwrap();
    let dflt = responses.find("default:").unwrap();
    assert!(c200 < c404 && c404 < c500, "{out}");
    assert!(c500 < dflt, "default sorts after numbers: {out}");
}

#[test]
fn paths_sort_by_specificity_then_alpha() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths:
  /pets/{id}/toys:
    get:
      operationId: a
      responses: {}
  /pets:
    get:
      operationId: b
      responses: {}
  /pets/{id}:
    get:
      operationId: c
      responses: {}
";
    let out = fmt(text);
    let paths = &out[out.find("paths:").unwrap()..];
    let static_path = paths.find("  /pets:").unwrap();
    let one_var = paths.find("  /pets/{id}:").unwrap();
    let two_var = paths.find("  /pets/{id}/toys:").unwrap();
    assert!(static_path < one_var && one_var < two_var, "{out}");
}

#[test]
fn extension_anchors_slot_at_their_positions() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
x-tagGroups: []
paths:
  /p:
    get:
      operationId: getP
      x-speakeasy-usage-example: true
      summary: S
      x-speakeasy-name-override: getP
      responses: {}
";
    let out = fmt(text);
    // Root: x-tagGroups right after `tags`-slot... tags absent, so it sits
    // after security/servers region — before paths.
    let root_tag_groups = out.find("x-tagGroups:").unwrap();
    let paths = out.find("paths:").unwrap();
    assert!(root_tag_groups < paths, "{out}");
    // Operation: name-override before operationId; usage-example after
    // deprecated (absent) → near summary.
    let name_override = out.find("x-speakeasy-name-override:").unwrap();
    let op_id = out.find("operationId:").unwrap();
    assert!(name_override < op_id, "{out}");
}

#[test]
fn example_subtrees_keep_author_order() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  examples:
    Weird:
      value:
        zebra: 1
        apple: 2
";
    let out = fmt(text);
    let zebra = out.find("zebra:").unwrap();
    let apple = out.find("apple:").unwrap();
    assert!(
        zebra < apple,
        "example payload order is presentation: {out}"
    );
}

#[test]
fn formatting_is_idempotent() {
    let text = "\
openapi: 3.1.0
paths:
  /pets/{id}:
    get:
      responses:
        '200': {description: ok, content: {application/json: {schema: {type: object}}}}
      summary: Get
      tags: [pets]
      operationId: getP
  /pets:
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema: {$ref: '#/components/schemas/Pet'}
      responses:
        '201': {description: made, content: {application/json: {schema: {$ref: '#/components/schemas/Pet'}}}}
info:
  title: T
  version: '1'
  contact:
    url: https://example.com
    name: Support
components:
  securitySchemes:
    apiKey: {type: http, scheme: bearer}
  schemas:
    Pet:
      required: [name]
      type: object
      properties:
        name: {type: string}
        kind: {type: string, enum: [dog, cat]}
";
    let once = fmt(text);
    let twice = canonical_format(&once, true, &ExtensionConfig::default());
    assert_eq!(
        once, twice,
        "second pass must be a no-op:\n{once}\n---\n{twice}"
    );
}

#[test]
fn comments_survive_the_full_pipeline_with_blocks_and_quotes() {
    let text = "\
openapi: 3.1.0
info:
  title: T  # title note
  version: '1'
paths:
  /p:
    get:
      # the operation id
      operationId: getP
      responses:
        '200':
          # response docs
          description: ok
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    # pet schema
    Pet:
      type: object
      description: |-
        Multi line
        block scalar
";
    let out = fmt(text);
    for comment in [
        "# the operation id",
        "# response docs",
        "# pet schema",
        "# title note",
    ] {
        assert!(out.contains(comment), "lost `{comment}`:\n{out}");
    }
    // Block scalar content survives verbatim.
    assert!(out.contains("Multi line\n        block scalar"), "{out}");
    // $ref now double-quoted.
    assert!(out.contains("$ref: \"#/components/schemas/Pet\""), "{out}");
    // The operation comment still precedes operationId, which now sits at
    // the top of the operation.
    let comment = out.find("# the operation id").unwrap();
    let op_id = out.find("operationId:").unwrap();
    assert!(comment < op_id, "{out}");
    // Re-parse must be clean.
    let uri = suspect_source::Uri::parse("mem://fmt-test.yaml").unwrap();
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(out.clone().into_bytes()),
    );
    assert!(
        low.syntax_errors().is_empty(),
        "output must stay valid YAML: {:?}\n{out}",
        low.syntax_errors()
    );
}

#[test]
fn sort_keys_disabled_preserves_order() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
zebra: last
alpha: first
";
    let out = canonical_format(text, false, &ExtensionConfig::default());
    let alpha = out.find("alpha:").unwrap();
    let zebra = out.find("zebra:").unwrap();
    assert!(
        zebra < alpha,
        "author order preserved when sorting off: {out}"
    );
}

#[test]
fn broken_documents_pass_through_untouched() {
    let text = "openapi: [broken\n  {\n";
    assert_eq!(fmt(text), text);
}

#[test]
fn registry_maps_sort_alphabetically() {
    let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
paths: {}
components:
  schemas:
    Zebra: {type: object}
    Apple: {type: object}
    Mango: {type: object}
";
    let out = fmt(text);
    let apple = out.find("    Apple:").unwrap();
    let mango = out.find("    Mango:").unwrap();
    let zebra = out.find("    Zebra:").unwrap();
    assert!(apple < mango && mango < zebra, "{out}");
}
