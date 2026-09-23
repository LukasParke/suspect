fn main() {
    let dir = std::env::temp_dir().join("suspect-lsp-sw-instances");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let spec = "swagger: \"2.0\"\ninfo: {title: t, version: '1'}\npaths:\n  /pets:\n    get:\n      operationId: listPets\n      parameters:\n        - name: limit\n          in: query\n          type: integer\n          minimum: 1\n          default: 0\n      responses:\n        '200': {description: ok}\ndefinitions:\n  Pet:\n    type: object\n    required: [name]\n    properties:\n      name: {type: string}\n";
    std::fs::write(dir.join("spec.yaml"), spec).unwrap();
    let uri = suspect_source::Uri::from_path(&dir.join("spec.yaml")).unwrap();
    let low = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(spec.as_bytes().to_vec()),
    );
    println!("family: {:?}", low.sniff_family());
    let diags = suspect_validate::validate_swagger_low(&low);
    for d in &diags {
        println!("{} :: {}", d.code, d.message);
    }
}
