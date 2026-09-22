package example.protocol;

import java.nio.file.*;
import java.util.*;
import static example.protocol.JsonRuntime.*;
import static example.protocol.Protocol.*;

/** Hand-authored normative wire strings checked against the actual Java runtime. */
public final class NativeNormative {
    private NativeNormative() {}
    private static void check(boolean condition,String message){if(!condition)throw new AssertionError(message);}
    public static void main(String[] args)throws Exception{
        JsonValue fixture=JsonRuntime.parse(Files.readAllBytes(Path.of(args[0])));int cases=0;
        for(JsonValue test:array(get(fixture,"parameters"))){
            String actual=HttpWire.parameter(get(test,"descriptor"),get(test,"value"),65536,new ModelCodec.Context());
            check(actual.equals(text(test,"expected")),text(test,"name")+": "+actual+" != "+text(test,"expected"));cases++;
        }
        JsonValue operation=get(fixture,"responses");
        for(JsonValue test:array(get(fixture,"responseCases"))){
            int status=(int)number(get(test,"status"));int response=HttpWire.chooseResponse(operation,status);JsonValue chosen=array(get(operation,"responses")).get(response);
            check(text(chosen,"status_key").equals(text(test,"response")),"response precedence");
            JsonValue expected=get(test,"media");if(expected!=JsonNull.INSTANCE){int media=HttpWire.chooseMedia(array(get(chosen,"media")),text(test,"contentType"));check(text(get(array(get(chosen,"media")).get(media),"media_type"),"declared").equals(text(expected)),"media specificity");}
            check((status>=200&&status<300)==flag(test,"success"),"actual status success");cases++;
        }
        System.out.println("JAVA_NORMATIVE_WIRE_OK: "+cases+" independent parameter/querystring/status/media vectors");
    }
}
