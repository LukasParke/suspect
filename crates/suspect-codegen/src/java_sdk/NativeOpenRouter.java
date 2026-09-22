import com.example.generated.*;
import com.example.generated.OpenRouter.*;
import static com.example.generated.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;
import com.sun.net.httpserver.HttpServer;

/** Installed consumer of five actual, unmodified OpenRouter operations. */
public final class Consumer {
    private Consumer() {}
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    public static void main(String[] args) throws Exception {
        JsonObject fixture = (JsonObject) JsonRuntime.parse(Files.readAllBytes(Path.of("openrouter-five-responses.json")));
        var replies = new HashMap<String, String>();
        fixture.values().forEach((name, value) -> replies.put(name, ((JsonString) value).value()));
        check(replies.size() == 5, "independent fixture count");
        var records = Collections.synchronizedList(new ArrayList<String>());
        var cookies = Collections.synchronizedList(new ArrayList<String>());
        var credits = new AtomicInteger();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        var executor = Executors.newVirtualThreadPerTaskExecutor(); server.setExecutor(executor);
        server.createContext("/", exchange -> {
            try {
                String uri = exchange.getRequestURI().toASCIIString();
                String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
                records.add(exchange.getRequestMethod() + " " + uri + " " + exchange.getRequestHeaders().getFirst("Authorization") + " " + body);
                cookies.add(String.valueOf(exchange.getRequestHeaders().getFirst("Cookie")));
                String response; int status = 200;
                if (uri.equals("/api/v1/credits")) {
                    if (credits.incrementAndGet() == 2) { status = 401; response = "{\"error\":{\"code\":401,\"message\":\"Missing Authentication header\"}}"; }
                    else response = replies.get("credits");
                } else if (uri.equals("/api/v1/keys")) { status = 201; response = replies.get("create"); }
                else if (uri.startsWith("/api/v1/keys/")) response = replies.get("update");
                else if (exchange.getRequestURI().getRawPath().endsWith("/files")) response = replies.get("list");
                else response = replies.get("file");
                byte[] bytes = response.getBytes(StandardCharsets.UTF_8);
                exchange.getResponseHeaders().add("Content-Type", "application/json; charset=utf-8");
                exchange.getResponseHeaders().add("Set-Cookie", "fixture=session; Path=/");
                exchange.sendResponseHeaders(status, bytes.length);
                exchange.getResponseBody().write(bytes);
            } finally { exchange.close(); }
        });
        server.start();
        URI base = URI.create("http://127.0.0.1:" + server.getAddress().getPort() + "/api/v1");
        try (var client = new OpenRouter(HttpRuntime.Options.builder().credential("apiKey", "test-management-token").serverUrl(base).build())) {
            var credit = client.getCredits();
            check(credit.status() == 200 && credit.data().data().totalCredits().token().equals("100.50000000000000001"), "exact credits");
            check(credit.data().data().totalUsage().token().equals("25.75"), "credit usage field");
            try { client.getCredits(); throw new AssertionError("401 became success"); }
            catch (GetCreditsStatus401 error) {
                check(error.status() == 401 && error.data().error().message().equals("Missing Authentication header"), "typed original API failure");
                check(!error.toString().contains("Missing Authentication"), "error leaked response body");
            }
            var createBody = CreateKeysRequest.builder("Native Test Key").limit(JsonNumber.parse("50.25")).limitReset(null).build();
            var createInput = CreateKeysInput.builder(createBody).build();
            var created = client.createKeys(createInput);
            check(created.status() == 201 && created.data().key().equals("fixture-secret"), "source create status and secret field");
            check(created.data().data().limit().token().equals("50.250"), "response number token");
            check(created.data().data().updatedAt() == null && created.data().data().externalUser() == null, "required response nulls");
            var updateBody = UpdateKeysRequest.builder().disabled(true).limit(JsonNumber.parse("75.50")).limitReset(null).name("Updated Native Key").build();
            var updateInput = UpdateKeysInput.builder("fixture-hash", updateBody).build();
            var updated = client.updateKeys(updateInput);
            check(updated.data().data().disabled() && updated.data().data().limit().token().equals("75.50"), "source update fields");
            var fileInput = GetContainerFileInput.builder("sess_abc123", "cfile_a/b 雪!'()*").build();
            var file = client.getContainerFile(fileInput).data();
            check(file.bytes().exactIntegerValue().intValueExact() == 123 && file.createdAt().exactIntegerValue().longValueExact() == 1755640000L, "mathematical integer fields");
            check(file.object().equals(ContainerFileObject.CONTAINER_FILE) && file.source().equals(ContainerFileSource.ASSISTANT), "native source literal fields");
            var listInput = ListContainerFilesInput.builder("sess_abc123").limit(JsonNumber.of(2)).after("a/b +雪").build();
            var page = client.listContainerFiles(listInput).data();
            check(page.data().size() == 1 && !page.hasMore() && page.firstId().equals("cfile-1"), "hand-authored list response");
            check(records.equals(List.of(
                "GET /api/v1/credits Bearer test-management-token ",
                "GET /api/v1/credits Bearer test-management-token ",
                "POST /api/v1/keys Bearer test-management-token {\"limit\":50.25,\"limit_reset\":null,\"name\":\"Native Test Key\"}",
                "PATCH /api/v1/keys/fixture-hash Bearer test-management-token {\"disabled\":true,\"limit\":75.50,\"limit_reset\":null,\"name\":\"Updated Native Key\"}",
                "GET /api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A Bearer test-management-token ",
                "GET /api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA Bearer test-management-token "
            )), "independent HTTP wire bytes: " + records);
            check(client.getCreditsAsync().get().data().data().totalCredits().token().equals("100.50000000000000001"), "async credits");
            check(client.createKeysAsync(createInput).get().status() == 201, "async create");
            check(client.updateKeysAsync(updateInput).get().status() == 200, "async update");
            check(client.getContainerFileAsync(fileInput).get().data().id().equals("cfile-1"), "async file");
            check(client.listContainerFilesAsync(listInput).get().data().data().size() == 1, "async list");
            client.listContainerFiles(ListContainerFilesInput.builder("sess_abc123").build());
            check(records.getLast().equals("GET /api/v1/containers/sess_abc123/files Bearer test-management-token "), "schema default inserted into absent query");

            String padded = "10e-" + "0".repeat(1000) + "1";
            var paddedFile = ContainerFile.decode(replies.get("file").replace("\"bytes\":123", "\"bytes\":" + padded));
            check(paddedFile.bytes().isInteger() && paddedFile.bytes().exactIntegerValue().intValueExact() == 1, "actual padded-exponent field regression");
            check(ContainerFile.encode(paddedFile).contains(padded), "actual field lost padded token");
            try { ContainerFile.decode(replies.get("file").replace("\"bytes\":123", "\"bytes\":0.1e" + "0".repeat(1000))); throw new AssertionError("fraction admitted as container bytes"); } catch (CodecException expected) { }
            var nullPage = ContainerFileListResponse.decode("{\"object\":\"list\",\"data\":[],\"first_id\":null,\"last_id\":null,\"has_more\":false}");
            check(nullPage.firstId() == null && nullPage.lastId() == null, "required nullable page fields");
            SdkExamples.getCreditsExample(client);
            SdkExamples.createKeysExample(client);
            SdkExamples.updateKeysExample(client);
            SdkExamples.getContainerFileExample(client);
            SdkExamples.listContainerFilesExample(client);
            GettingStarted.run(client);
            check(cookies.stream().allMatch("null"::equals), "default transport persisted a response cookie");
        } finally { server.stop(0); executor.close(); }
        System.out.println("JAVA_OPENROUTER_OK: five actual operations, 5 independent response fixtures, sync/async, builders, docs calls and exact fields");
    }
}
