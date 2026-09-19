import example.openrouterprotocol.*;
import example.openrouterprotocol.Client.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import com.sun.net.httpserver.HttpServer;

/** Newly admitted actual OpenRouter operations, with independent HTTP responses. */
public final class NativeProtocolOpenRouter {
    private NativeProtocolOpenRouter() {}
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }
    public static void main(String[] args) throws Exception {
        var server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        var executor = Executors.newVirtualThreadPerTaskExecutor(); server.setExecutor(executor);
        var requests = Collections.synchronizedList(new ArrayList<String>());
        server.createContext("/api/v1/", exchange -> {
            try {
                String path = exchange.getRequestURI().getRawPath(); requests.add(exchange.getRequestURI().toASCIIString());
                int status = 200; String media = "application/octet-stream"; byte[] body = new byte[] {0, (byte) 255, 13, 10, 1};
                if (path.equals("/api/v1/credits/coinbase")) {
                    check(exchange.getRequestMethod().equals("POST") && exchange.getRequestHeaders().getFirst("Authorization") == null, "anonymous source operation acquired auth");
                    status = 410; media = "application/json"; body = "{\"error\":{\"code\":410,\"message\":\"deprecated\"},\"user_id\":null}".getBytes(StandardCharsets.UTF_8);
                } else {
                    check(exchange.getRequestMethod().equals("GET") && "Bearer native-token".equals(exchange.getRequestHeaders().getFirst("Authorization")), "source bearer attachment");
                    if (path.equals("/api/v1/files/deny/content")) { status = 404; media = "application/json"; body = "{\"error\":{\"code\":404,\"message\":\"missing\"}}".getBytes(StandardCharsets.UTF_8); }
                }
                exchange.getResponseHeaders().add("Content-Type", media); exchange.sendResponseHeaders(status, body.length); exchange.getResponseBody().write(body);
            } finally { exchange.close(); }
        });
        server.start();
        try (var client = new Client(HttpRuntime.Options.builder().serverUrl(URI.create("http://127.0.0.1:" + server.getAddress().getPort() + "/api/v1")).credential("apiKey", "native-token").build())) {
            var container = client.downloadContainerFileContent(DownloadContainerFileContentInput.builder("sess_abc123", "cfile_bytes").build());
            check(Arrays.equals(container.data().toByteArray(), new byte[] {0, (byte) 255, 13, 10, 1}), "container byte content became JSON/text");
            var file = client.downloadFileContentAsync(DownloadFileContentInput.builder("or_file_native").workspaceId("native workspace").build()).get();
            byte[] copy = file.data().toByteArray(); copy[0] = 9;
            check(file.data().toByteArray()[0] == 0, "binary output not immutable");
            try { client.downloadFileContent(DownloadFileContentInput.builder("deny").build()); throw new AssertionError("missing file became success"); }
            catch (DownloadFileContentStatus404 error) { check(error.status() == 404 && error.data().error().message().equals("missing"), "typed source JSON failure"); }
            try { client.createCoinbaseCharge(); throw new AssertionError("deprecated endpoint became success"); }
            catch (CreateCoinbaseChargeStatus410 error) { check(error.status() == 410 && error.data().error().code().exactIntegerValue().intValueExact() == 410 && error.data().userId().isPresent() && error.data().userId().value() == null, "typed gone response and nullable metadata"); }
            check(requests.equals(List.of("/api/v1/containers/sess_abc123/files/cfile_bytes/content", "/api/v1/files/or_file_native/content?workspace_id=native%20workspace", "/api/v1/files/deny/content", "/api/v1/credits/coinbase")), "actual operation paths: " + requests);
        } finally { server.stop(0); executor.close(); }
        System.out.println("JAVA_PROTOCOL_OPENROUTER_OK: three new source operations, binary downloads, anonymous call and typed 404/410");
    }
}
