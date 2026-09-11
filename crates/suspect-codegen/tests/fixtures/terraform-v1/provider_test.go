// Independent native lifecycle controls, installed as a test-only package overlay.
package provider

import (
 "context"
 "errors"
 "io"
 "net/http"
 "net/http/httptest"
 "strings"
 "sync/atomic"
 "testing"
 "time"

 "github.com/hashicorp/terraform-plugin-framework/provider"
 "github.com/hashicorp/terraform-plugin-framework/resource"
 "github.com/hashicorp/terraform-plugin-framework/tfsdk"
 "github.com/hashicorp/terraform-plugin-framework/types"
 sdk "example.com/lifecycle-sdk"
)

// This independent model deliberately does not use the emitter's allocated
// Go field names. The real Framework schema must bind its tfsdk tags.
type fixtureModel struct {
 Description types.String `tfsdk:"description"`
 Enabled types.Bool `tfsdk:"enabled"`
 Fingerprint types.String `tfsdk:"fingerprint"`
 ID types.String `tfsdk:"id"`
 Memo types.String `tfsdk:"memo"`
 Name types.String `tfsdk:"name"`
 Region types.String `tfsdk:"region"`
 Secret types.String `tfsdk:"secret"`
 SecretVersion types.String `tfsdk:"secret_version"`
}

func configured() fixtureModel {
 return fixtureModel{Description:types.StringNull(), Enabled:types.BoolValue(false), Fingerprint:types.StringUnknown(), ID:types.StringUnknown(), Memo:types.StringNull(), Name:types.StringValue("alpha"), Region:types.StringValue("east"), Secret:types.StringNull(), SecretVersion:types.StringValue("1")}
}

func requestData(t *testing.T, r resource.Resource, m fixtureModel) (tfsdk.Plan, tfsdk.Config, tfsdk.State) {
 t.Helper()
 var schema resource.SchemaResponse
 r.Schema(context.Background(), resource.SchemaRequest{}, &schema)
 state := tfsdk.State{Schema:schema.Schema}
 if d:=state.Set(context.Background(), m); d.HasError() { t.Fatal(d) }
 return tfsdk.Plan{Schema:schema.Schema, Raw:state.Raw}, tfsdk.Config{Schema:schema.Schema, Raw:state.Raw}, state
}

func clientResource(t *testing.T, transport sdk.Doer, endpoint string) resource.Resource {
 t.Helper()
 p := NewWithTransport("0.1.0", transport)()
 var schema provider.SchemaResponse
 p.Schema(context.Background(), provider.SchemaRequest{}, &schema)
 state := tfsdk.State{Schema:schema.Schema}
 data := struct { Endpoint types.String `tfsdk:"endpoint"`; Token types.String `tfsdk:"token"` }{types.StringValue(endpoint), types.StringValue("fixture-token")}
 if d:=state.Set(context.Background(), data); d.HasError() { t.Fatal(d) }
 var response provider.ConfigureResponse
 p.Configure(context.Background(), provider.ConfigureRequest{Config:tfsdk.Config{Schema:schema.Schema, Raw:state.Raw}}, &response)
 if response.Diagnostics.HasError() { t.Fatal(response.Diagnostics) }
 r := p.Resources(context.Background())[0]()
 var configured resource.ConfigureResponse
 r.(resource.ResourceWithConfigure).Configure(context.Background(), resource.ConfigureRequest{ProviderData:response.ResourceData}, &configured)
 if configured.Diagnostics.HasError() { t.Fatal(configured.Diagnostics) }
 t.Cleanup(response.ResourceData.(*sdk.Client).CloseIdleConnections)
 return r
}

type doerFunc func(*http.Request) (*http.Response, error)
func (f doerFunc) Do(r *http.Request) (*http.Response,error) { return f(r) }

const record = `{"SetExtra":"alpha","credential-fingerprint":"sensitive-fingerprint","description":null,"enabled":false,"id":"opaque/a 雪","region":"east"}`

func TestNativeSchemaWireUnknownNullAndSDKValidation(t *testing.T) {
 var calls atomic.Int32
 server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
  calls.Add(1)
  body,_:=io.ReadAll(r.Body)
  expected := `{"SetExtra":"alpha","description":null,"enabled":false,"region":"east","secret":"write-only-secret"}`
  if r.Method!="POST" || r.RequestURI!="/v1/records" || r.Header.Get("Authorization")!="Bearer fixture-token" || r.Header.Get("Content-Type")!="application/json" || string(body)!=expected { t.Errorf("wire mismatch: %s %s %q %q %q",r.Method,r.RequestURI,r.Header.Get("Authorization"),r.Header.Get("Content-Type"),body) }
  w.Header().Set("Content-Type","application/json"); w.WriteHeader(201); _,_=io.WriteString(w,record)
 })); defer server.Close()
 r := clientResource(t, server.Client(), server.URL+"/v1")
 desired:=configured()
 plan,config,state:=requestData(t,r,desired)
 configData:=desired; configData.Secret=types.StringValue("write-only-secret")
 _,config,_=requestData(t,r,configData)
 response:=resource.CreateResponse{State:tfsdk.State{Schema:state.Schema}}
 r.Create(context.Background(),resource.CreateRequest{Plan:plan,Config:config},&response)
 if response.Diagnostics.HasError() { t.Fatal(response.Diagnostics) }
 var got fixtureModel
 if d:=response.State.Get(context.Background(),&got); d.HasError() { t.Fatal(d) }
 if got.ID.ValueString()!="opaque/a 雪" || !got.Description.IsNull() || !got.Memo.IsNull() || !got.Secret.IsNull() || got.Enabled.IsUnknown() || got.Enabled.ValueBool() || got.Fingerprint.ValueString()!="sensitive-fingerprint" { t.Fatalf("state/null/secret mismatch: %#v",got) }
 if calls.Load()!=1 { t.Fatal("create called API more than once") }
 for _,which:=range []string{"unknown-required","unknown-optional","null-required","invalid-sdk-string","invalid-sdk-enum"} {
  t.Run(which,func(t *testing.T) {
   bad:=desired
   switch which { case "unknown-required": bad.Name=types.StringUnknown(); case "unknown-optional": bad.Memo=types.StringUnknown(); case "null-required": bad.Name=types.StringNull(); case "invalid-sdk-string": bad.Name=types.StringValue(""); case "invalid-sdk-enum": bad.Region=types.StringValue("not-a-region") }
   p,c,s:=requestData(t,r,bad)
   rejected:=resource.CreateResponse{State:tfsdk.State{Schema:s.Schema}}
   r.Create(context.Background(),resource.CreateRequest{Plan:p,Config:c},&rejected)
   if !rejected.Diagnostics.HasError() || calls.Load()!=1 { t.Fatalf("invalid/unknown input reached SDK transport: %s %#v",which,rejected.Diagnostics) }
  })
 }
}

func TestSDKContextPropagatesThroughFrameworkReadAndCancelsBody(t *testing.T) {
 started:=make(chan struct{}); closed:=make(chan struct{})
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request) {
  if r.RequestURI!="/v1/records/opaque%2Fa%20%E9%9B%AA" { t.Errorf("encoded import ID lost: %s",r.RequestURI) }
  w.Header().Set("Content-Type","application/json"); _,_=io.WriteString(w,"{"); w.(http.Flusher).Flush(); close(started); <-r.Context().Done(); close(closed)
 })); defer server.Close()
 type contextKey struct{}
 marker:=&struct{}{}
 transport:=doerFunc(func(req *http.Request)(*http.Response,error) {
  if req.Context().Value(contextKey{})!=marker { return nil,errors.New("Terraform context value was dropped") }
  return server.Client().Do(req)
 })
 r:=clientResource(t,transport,server.URL+"/v1")
 old:=configured(); old.ID=types.StringValue("opaque/a 雪"); old.Fingerprint=types.StringValue("known")
 _,_,state:=requestData(t,r,old)
 ctx,cancel:=context.WithCancel(context.WithValue(context.Background(),contextKey{},marker)); defer cancel()
 done:=make(chan resource.ReadResponse,1)
 go func(){ response:=resource.ReadResponse{State:state}; r.Read(ctx,resource.ReadRequest{State:state},&response); done<-response }()
 select { case <-started: case <-time.After(5*time.Second): t.Fatal("SDK call never started") }
 cancel()
 select { case response:=<-done: if !response.Diagnostics.HasError() || !strings.Contains(response.Diagnostics[0].Detail(),"cancelled") { t.Fatal(response.Diagnostics) }; case <-time.After(5*time.Second): t.Fatal("provider detached Terraform cancellation") }
 select { case <-closed: case <-time.After(5*time.Second): t.Fatal("SDK response body remained open") }
}

func TestUpdateTriggerPartialAndFailedOperations(t *testing.T) {
 old:=configured(); old.ID=types.StringValue("opaque/a 雪"); old.Fingerprint=types.StringValue("sensitive-fingerprint")
 var calls atomic.Int32
 var status atomic.Int32; status.Store(200)
 var expectSecret atomic.Bool
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request) {
  calls.Add(1); body,_:=io.ReadAll(r.Body)
  if r.Method!="PUT" || r.RequestURI!="/v1/records/opaque%2Fa%20%E9%9B%AA" { t.Errorf("wrong update route: %s %s",r.Method,r.RequestURI) }
  want:=`{"SetExtra":"alpha","description":null,"enabled":false}`
  if expectSecret.Load() { want=`{"SetExtra":"alpha","description":null,"enabled":false,"secret":"rotated-secret"}` }
  if string(body)!=want { t.Errorf("trigger/null/omission mismatch: %s",body) }
  w.Header().Set("Content-Type","application/json"); w.WriteHeader(int(status.Load()))
  if status.Load()==409 { _,_=io.WriteString(w,`{"message":"secret text must not appear in diagnostics"}`) } else { _,_=io.WriteString(w,record) }
 })); defer server.Close()
 r:=clientResource(t,server.Client(),server.URL+"/v1")
 _,_,prior:=requestData(t,r,old)
 for _,test:=range []struct{name string; version string; secret types.String; status int32; sent bool; error bool; retainedVersion string}{
  {"unchanged-trigger","1",types.StringValue("must-not-send"),200,false,false,"1"},
  {"changed-trigger","2",types.StringValue("rotated-secret"),200,true,false,"2"},
  {"partial","2",types.StringValue("rotated-secret"),503,true,true,"1"},
  {"failed","2",types.StringValue("rotated-secret"),409,true,true,"1"},
  {"missing-secret","2",types.StringNull(),200,false,true,"1"},
  {"unknown-secret","2",types.StringUnknown(),200,false,true,"1"},
  {"unknown-trigger","2",types.StringValue("rotated-secret"),200,false,true,"1"},
 } {
  t.Run(test.name,func(t *testing.T){
   desired:=old; desired.SecretVersion=types.StringValue(test.version)
   if test.name=="unknown-trigger" { desired.SecretVersion=types.StringUnknown() }
   p,_,_:=requestData(t,r,desired); configData:=desired; configData.Secret=test.secret; _,c,_:=requestData(t,r,configData)
   status.Store(test.status); expectSecret.Store(test.sent); before:=calls.Load()
   response:=resource.UpdateResponse{State:prior}
   r.Update(context.Background(),resource.UpdateRequest{Plan:p,Config:c,State:prior},&response)
   if response.Diagnostics.HasError()!=test.error { t.Fatal(response.Diagnostics) }
   var got fixtureModel; if d:=response.State.Get(context.Background(),&got); d.HasError(){ t.Fatal(d) }
   if !got.Secret.IsNull() || got.SecretVersion.ValueString()!=test.retainedVersion || got.ID.ValueString()!=old.ID.ValueString() { t.Fatalf("partial state/secret policy lost: %#v",got) }
   for _,d:=range response.Diagnostics { if strings.Contains(d.Detail(),"secret text") || strings.Contains(d.Detail(),"rotated-secret") { t.Fatal("API response/secret leaked into diagnostics") } }
   if (test.name=="missing-secret" || strings.HasPrefix(test.name,"unknown-")) && calls.Load()!=before { t.Fatal("null/unknown write-only value or trigger sent as a default") }
  })
 }
}

func TestMissingUsesValidatedDeclaredSDKErrorAndFailedDeleteKeepsState(t *testing.T) {
 for _,test:=range []struct{status int; payload string; removed bool}{
  {404,`{"message":"missing"}`,true},
  {404,`{"message":42}`,false},
  {409,`{"message":"blocked"}`,false},
  {418,`{"message":"undeclared"}`,false},
 } {
  server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request) { w.Header().Set("Content-Type","application/json"); w.WriteHeader(test.status); _,_=io.WriteString(w,test.payload) }))
  r:=clientResource(t,server.Client(),server.URL+"/v1")
  old:=configured(); old.ID=types.StringValue("a"); old.Fingerprint=types.StringValue("f"); _,_,state:=requestData(t,r,old)
  response:=resource.DeleteResponse{State:state}; r.Delete(context.Background(),resource.DeleteRequest{State:state},&response)
  if response.State.Raw.IsNull()!=test.removed || response.Diagnostics.HasError()==test.removed { t.Fatalf("status %d bypassed SDK error decoding: %#v",test.status,response.Diagnostics) }
  server.Close()
 }
}
