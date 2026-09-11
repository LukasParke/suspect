package provider

import (
 "context"
 "io"
 "net/http"
 "strings"
 "sync/atomic"
 "testing"

 "github.com/hashicorp/terraform-plugin-framework/datasource"
 "github.com/hashicorp/terraform-plugin-framework/resource"
 "github.com/hashicorp/terraform-plugin-framework/tfsdk"
 "github.com/hashicorp/terraform-plugin-framework/types"
 sdk "example.com/lifecycle-sdk"
)

type trackedErrorBody struct { reader io.Reader; reads, closes atomic.Int32 }
func(b *trackedErrorBody)Read(p []byte)(int,error){ b.reads.Add(1); return b.reader.Read(p) }
func(b *trackedErrorBody)Close()error{ b.closes.Add(1); return nil }

func TestAllDeferredAPIErrorsAreReleasedWithoutChangingState(t *testing.T) {
 for _,phase:=range []string{"create","read","update","delete","data"} {
  statuses:=[]int{409};if phase=="read"||phase=="delete"||phase=="data"{statuses=append(statuses,503)}
  for _,status:=range statuses {
  t.Run(phase+"/"+http.StatusText(status),func(t *testing.T){
   body:=&trackedErrorBody{reader:strings.NewReader("malformed-jsonl-secret\n")}
   type key struct{}
   ctx:=context.WithValue(context.Background(),key{},"terraform-context")
   r:=clientResource(t,doerFunc(func(req *http.Request)(*http.Response,error){
    if req.Context().Value(key{})!="terraform-context" { t.Error("Terraform context lost") }
    return &http.Response{StatusCode:status,Header:http.Header{"Content-Type":[]string{"application/jsonl"}},Body:body,Request:req},nil
   }),"http://127.0.0.1:1/v1")
   old:=configured(); old.ID=types.StringValue("opaque/a 雪"); old.Fingerprint=types.StringValue("known")
   plan,config,prior:=requestData(t,r,old)
   switch phase {
   case "create":
    response:=resource.CreateResponse{State:tfsdk.State{Schema:prior.Schema}}; r.Create(ctx,resource.CreateRequest{Plan:plan,Config:config},&response)
    if !response.Diagnostics.HasError() || !response.State.Raw.IsNull() { t.Fatal("failed create invented state",response.Diagnostics) }
   case "read":
    response:=resource.ReadResponse{State:prior}; r.Read(ctx,resource.ReadRequest{State:prior},&response)
    if !response.Diagnostics.HasError() || !response.State.Raw.Equal(prior.Raw) { t.Fatal("unmapped error changed read state",response.Diagnostics) }
   case "update":
    response:=resource.UpdateResponse{State:prior}; r.Update(ctx,resource.UpdateRequest{Plan:plan,Config:config,State:prior},&response)
    if !response.Diagnostics.HasError() || !response.State.Raw.Equal(prior.Raw) { t.Fatal("unmapped error changed update state",response.Diagnostics) }
   case "delete":
    response:=resource.DeleteResponse{State:prior}; r.Delete(ctx,resource.DeleteRequest{State:prior},&response)
    if !response.Diagnostics.HasError() || !response.State.Raw.Equal(prior.Raw) { t.Fatal("unmapped error removed state",response.Diagnostics) }
   case "data":
    d:=NewDataSource0(); var c datasource.ConfigureResponse
    d.(datasource.DataSourceWithConfigure).Configure(ctx,datasource.ConfigureRequest{ProviderData:r.(*Resource0).client},&c)
    var s datasource.SchemaResponse; d.Schema(ctx,datasource.SchemaRequest{},&s)
    type input struct { ID types.String `tfsdk:"id"`; Name types.String `tfsdk:"name"`; Region types.String `tfsdk:"region"`; Enabled types.Bool `tfsdk:"enabled"`; Description types.String `tfsdk:"description"`; Memo types.String `tfsdk:"memo"`; Fingerprint types.String `tfsdk:"fingerprint"` }
    state:=tfsdk.State{Schema:s.Schema}; if x:=state.Set(ctx,input{ID:types.StringValue("opaque/a 雪")}); x.HasError(){t.Fatal(x)}
    response:=datasource.ReadResponse{State:tfsdk.State{Schema:s.Schema}}; d.Read(ctx,datasource.ReadRequest{Config:tfsdk.Config{Schema:s.Schema,Raw:state.Raw}},&response)
    if !response.Diagnostics.HasError() || !response.State.Raw.IsNull() { t.Fatal("unmapped data-source error invented state",response.Diagnostics) }
   }
   t.Logf("phase=%s status=%d reads=%d closes=%d caller_context_error=%v",phase,status,body.reads.Load(),body.closes.Load(),ctx.Err())
   if body.closes.Load()!=1 || body.reads.Load()!=0 || ctx.Err()!=nil { t.Fatal("SDK error stream ownership/context changed") }
  })
  }
 }
}

func TestBufferedMissingAndPartialErrorsKeepSDKValidationBoundary(t *testing.T) {
 for _,test:=range []struct{phase string;status int;payload string;removed bool;partial bool;failure bool}{
  {"read",404,`{"message":"missing"}`,true,false,false},
  {"delete",404,`{"message":"missing"}`,true,false,false},
  {"read",404,`{"message":42}`,false,false,true},
  {"delete",404,`{"message":42}`,false,false,true},
  {"create",503,record,false,true,true},
  {"update",503,record,false,true,true},
  {"create",503,`{"id":"unknown-side-effect"}`,true,false,true},
  {"update",503,`{"id":"unknown-side-effect"}`,false,false,true},
  {"update",503,strings.Replace(record,"opaque/a 雪","changed-id",1),false,false,true},
 } {
  t.Run(test.phase+"/"+http.StatusText(test.status)+"/"+test.payload,func(t *testing.T){
   body:=&trackedErrorBody{reader:strings.NewReader(test.payload)}
   r:=clientResource(t,doerFunc(func(req *http.Request)(*http.Response,error){return &http.Response{StatusCode:test.status,Header:http.Header{"Content-Type":[]string{"application/json"}},Body:body,Request:req},nil}),"http://127.0.0.1:1/v1")
   old:=configured(); old.ID=types.StringValue("opaque/a 雪"); old.Name=types.StringValue("prior"); old.Fingerprint=types.StringValue("known")
   desired:=old;desired.Name=types.StringValue("alpha"); plan,config,_:=requestData(t,r,desired);_,_,prior:=requestData(t,r,old)
   var state tfsdk.State; var failure bool
   switch test.phase {
   case "read": out:=resource.ReadResponse{State:prior};r.Read(context.Background(),resource.ReadRequest{State:prior},&out);state=out.State;failure=out.Diagnostics.HasError()
   case "delete": out:=resource.DeleteResponse{State:prior};r.Delete(context.Background(),resource.DeleteRequest{State:prior},&out);state=out.State;failure=out.Diagnostics.HasError()
   case "create": out:=resource.CreateResponse{State:tfsdk.State{Schema:prior.Schema}};r.Create(context.Background(),resource.CreateRequest{Plan:plan,Config:config},&out);state=out.State;failure=out.Diagnostics.HasError()
   case "update": out:=resource.UpdateResponse{State:prior};r.Update(context.Background(),resource.UpdateRequest{Plan:plan,Config:config,State:prior},&out);state=out.State;failure=out.Diagnostics.HasError()
   }
   if failure!=test.failure || state.Raw.IsNull()!=test.removed || body.closes.Load()!=1 || body.reads.Load()==0 { t.Fatalf("SDK decode/state/cleanup mismatch failure=%v null=%v reads=%d closes=%d",failure,state.Raw.IsNull(),body.reads.Load(),body.closes.Load()) }
   if test.partial { var actual fixtureModel; if d:=state.Get(context.Background(),&actual);d.HasError(){t.Fatal(d)};if actual.Name.ValueString()!="alpha"||actual.ID.ValueString()!=old.ID.ValueString(){t.Fatal("partial decoded state lost")} } else if !test.removed && !state.Raw.Equal(prior.Raw) { t.Fatal("failed codec/operation changed prior state") }
  })
 }
}

func TestTransportCauseCannotMasqueradeAsMissingSDKResponse(t *testing.T) {
 r:=clientResource(t,doerFunc(func(*http.Request)(*http.Response,error){ return nil,&sdk.ReadItem2Status404{} }),"http://127.0.0.1:1/v1")
 old:=configured();old.ID=types.StringValue("opaque/a 雪");old.Fingerprint=types.StringValue("known");_,_,prior:=requestData(t,r,old)
 out:=resource.ReadResponse{State:prior};r.Read(context.Background(),resource.ReadRequest{State:prior},&out)
 if !out.Diagnostics.HasError() || !out.State.Raw.Equal(prior.Raw) { t.Fatal("transport cause was reclassified as an actual missing response") }
}

func TestCancellationAlsoReleasesReturnedErrorBody(t *testing.T) {
 body:=&trackedErrorBody{reader:strings.NewReader("malformed-jsonl\n")}
 ctx,cancel:=context.WithCancel(context.Background());defer cancel()
 r:=clientResource(t,doerFunc(func(req *http.Request)(*http.Response,error){
  cancel()
  return &http.Response{StatusCode:503,Header:http.Header{"Content-Type":[]string{"application/jsonl"}},Body:body,Request:req},nil
 }),"http://127.0.0.1:1/v1")
 old:=configured();old.ID=types.StringValue("opaque/a 雪");old.Fingerprint=types.StringValue("known");_,_,prior:=requestData(t,r,old)
 out:=resource.ReadResponse{State:prior};r.Read(ctx,resource.ReadRequest{State:prior},&out)
 if ctx.Err()!=context.Canceled || !out.Diagnostics.HasError() || !out.State.Raw.Equal(prior.Raw) || body.closes.Load()!=1 {t.Fatalf("cancellation ownership failed context=%v diagnostics=%v closes=%d",ctx.Err(),out.Diagnostics,body.closes.Load())}
}
