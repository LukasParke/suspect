//! Static metadata loading does not enlarge the public JSON payload policy.
#![cfg(feature = "java-sdk")]
use std::process::Command;

#[test]
#[ignore = "requires native JDK 21 or newer"]
fn native_generated_metadata_uses_a_separate_finite_strict_json_budget() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("JsonRuntime.java"),
        include_str!("../src/java_sdk/JsonRuntime.java").replace("{package}", "example"),
    )
    .unwrap();
    std::fs::write(root.join("MetadataBudget.java"), r#"package example;
import java.nio.charset.StandardCharsets;
public final class MetadataBudget {
    private MetadataBudget() {}
    private static void rejects(Runnable action) {
        try { action.run(); } catch (JsonRuntime.JsonError expected) { return; }
        throw new AssertionError("strict finite JSON boundary accepted invalid input");
    }
    public static void main(String[] args) {
        String member = "x".repeat(JsonRuntime.MAX_BYTES / 2);
        byte[] metadata = ("[\"" + member + "\",\"" + member + "\"]").getBytes(StandardCharsets.UTF_8);
        var value = (JsonRuntime.JsonArray) JsonRuntime.parseProgram(metadata);
        if (value.values().size() != 2) throw new AssertionError("metadata lost members");
        rejects(() -> JsonRuntime.parse(metadata));
        rejects(() -> JsonRuntime.parseProgram(new byte[JsonRuntime.MAX_PROGRAM_BYTES + 1]));
        rejects(() -> JsonRuntime.parseProgram(new byte[]{(byte)0xc0, (byte)0xaf}));
        rejects(() -> JsonRuntime.parseProgram("{\"x\":1,\"x\":2}".getBytes(StandardCharsets.UTF_8)));
        System.out.println("large metadata and independent strict payload/program limits passed");
    }
}
"#).unwrap();
    let javac = std::env::var_os("SUSPECT_JAVAC_BIN").unwrap_or_else(|| "javac".into());
    let java = std::env::var_os("SUSPECT_JAVA_BIN").unwrap_or_else(|| "java".into());
    for (binary, arguments) in [
        (
            javac,
            vec![
                "--release",
                "21",
                "-Xlint:all",
                "-Werror",
                "-d",
                ".",
                "JsonRuntime.java",
                "MetadataBudget.java",
            ],
        ),
        (java, vec!["-Xmx512m", "-cp", ".", "example.MetadataBudget"]),
    ] {
        let output = Command::new(binary)
            .args(arguments)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
