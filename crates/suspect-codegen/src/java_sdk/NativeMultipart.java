import example.protocol.*;
import example.protocol.Client.*;
import static example.protocol.JsonRuntime.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Literal MIME witnesses for typed multipart, styles, extras and structural rules. */
public final class NativeMultipart {
    private NativeMultipart() {}
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }
    private static SdkException fails(String kind, Runnable action) { return NativeProtocolControls.fails(kind, action); }
    private static String part(String name, String content) {
        return "--edge\r\nContent-Disposition: form-data; name=\"" + name + "\"\r\n\r\n" + content + "\r\n";
    }
    private static String styles() {
        return part("list", "list=blue&list=black") + part("color", "g=2&r=1") + part("deep", "deep%5Bg%5D=2&deep%5Br%5D=1")
                + part("pipe", "pipe=a%7Cb") + part("space", "space=a%20b") + part("plain", "plain=  a=b&c + %20  ") + "--edge--\r\n";
    }
    private static StyledPartsInput input() {
        return StyledPartsInput.builder(StyledPartsMultipart.builder(
                StyledPartsMultipartColorPart.builder(ComponentsRequestBodiesStyledMultipartFormDataColor.builder(JsonNumber.of(2), JsonNumber.of(1)).build()).build(),
                StyledPartsMultipartDeepPart.builder(ComponentsRequestBodiesStyledMultipartFormDataDeep.builder(JsonNumber.of(2), JsonNumber.of(1)).build()).build(),
                StyledPartsMultipartListPart.builder(List.of("blue", "black")).build(),
                StyledPartsMultipartPipePart.builder(List.of("a", "b")).build(),
                StyledPartsMultipartPlainPart.builder("  a=b&c + %20  ").build(),
                StyledPartsMultipartSpacePart.builder(List.of("a", "b")).build()).build()).build();
    }
    private static NativeSupport.Mock transport(String body) { return new NativeSupport.Mock(new NativeSupport.Script(200, "multipart/form-data; boundary=\"edge\"", body)); }
    private static Client client(NativeSupport.Mock transport) { return new Client(HttpRuntime.Options.builder().httpClient(transport).build()); }
    private static void stylesAndExactOctets() {
        var transport = transport("preamble\r\n" + styles() + "epilogue");
        try (var client = client(transport)) {
            var result = client.styledParts(input()).data();
            check(result.list().size() == 1 && result.list().getFirst().value().equals(List.of("blue", "black")), "exploded array part was not decoded");
            check(result.color().value().r().exactIntegerValue().intValueExact() == 1 && result.deep().value().g().exactIntegerValue().intValueExact() == 2, "exploded/deep object part");
            check(result.pipe().getFirst().value().equals(List.of("a", "b")) && result.space().getFirst().value().equals(List.of("a", "b")), "delimited part arrays");
            check(result.plain().value().equals("  a=b&c + %20  "), "multipart style must retain literal whitespace, plus and percent data");
            String sent = new String(transport.requests.getLast().body(), StandardCharsets.UTF_8);
            check(sent.contains("\r\n\r\nlist=blue&list=black\r\n") && sent.contains("\r\n\r\ng=2&r=1\r\n") && sent.contains("\r\n\r\ndeep%5Bg%5D=2&deep%5Br%5D=1\r\n"), "independent multipart style request bytes");
        }
        try (var client = client(transport("preamble --edge is not a delimiter\r\n" + styles()))) {
            check(client.styledParts(input()).data().list().getFirst().value().size() == 2, "a boundary-like preamble string is not a MIME delimiter");
        }
    }
    private static void typedExtras() {
        var builder = ExtraPartsMultipart.builder(ExtraPartsMultipartBPart.builder("base").build())
                .putAdditionalPart("required-extra", ExtraPartsMultipartAdditionalPart.builder("extra").build());
        var snapshot = builder.build(); builder.additional2(ExtraPartsMultipartAdditional2Part.builder("changed").build());
        check(snapshot.additionalParts().size() == 1 && !snapshot.additional2().isPresent(), "extra map was not snapshotted");
        fails("missing-required-part", () -> ExtraPartsMultipart.builder(ExtraPartsMultipartBPart.builder("base").build()).additional2(ExtraPartsMultipartAdditional2Part.builder("not-the-required-extra").build()).build());
        fails("wire-cardinality", () -> builder.putAdditionalPart("too-many", ExtraPartsMultipartAdditionalPart.builder("extra").build()).build());
        try { builder.putAdditionalPart("b", ExtraPartsMultipartAdditionalPart.builder("bad").build()); throw new AssertionError("extra shadows declared part"); }
        catch (IllegalArgumentException expected) { }
        try (var client = client(transport(part("b", "base") + part("required-extra", "extra") + "--edge--\r\n"))) {
            var result = client.extraParts(ExtraPartsInput.builder(snapshot).build()).data();
            check(result.b().value().equals("base") && result.additionalParts().get("required-extra").value().equals("extra"), "source-typed additional part round trip");
        }
    }
    private static void invalidMime() {
        for (String body : List.of(styles().replace("list=blue&list=black", "list=blue&wrong=black"), styles().replace("deep%5Bg%5D=2", "wrong%5Bg%5D=2"))) {
            try (var client = client(transport(body))) { check(fails("invalid-part-field", () -> client.styledParts(input())).status() == 200, "part error lost actual status"); }
        }
        try (var client = client(transport(part("plain", "plain=duplicate") + styles()))) { fails("invalid-part-multiplicity", () -> client.styledParts(input())); }
        try (var client = client(transport(styles().replace("--edge--\r\n", "")))) { fails("unterminated-multipart", () -> client.styledParts(input())); }
        try (var client = client(transport(styles().replace("name=\"plain\"\r\n", "name=\"plain\"\r\nContent-Transfer-Encoding: base64\r\n")))) { fails("unexpected-part-transfer-encoding", () -> client.styledParts(input())); }
        String binary = "--edge\r\nContent-Disposition: form-data; name=\"file\"\r\nContent-Type: application/octet-stream\r\nX-Size: 33\r\n\r\n" + "x".repeat(33) + "\r\n" + part("title", "title") + "--edge--\r\n";
        try (var client = client(transport(binary))) { fails("resource-limit", client::downloadParts); }
        try (var client = client(transport(part("title", "title") + "--edge--\r\n"))) { fails("missing-required-part", client::downloadParts); }
        try (var client = client(transport(part("unknown", "x") + "--edge--\r\n"))) { fails("undeclared-part", client::downloadParts); }
    }
    public static void main(String[] args) {
        stylesAndExactOctets(); typedExtras(); invalidMime();
        System.out.println("JAVA_MULTIPART_OK: literal styles, typed extras, bytes, structural rules and malformed MIME");
    }
}
