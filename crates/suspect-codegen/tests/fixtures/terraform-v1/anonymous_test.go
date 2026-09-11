package provider

import (
 "context"
 "io"
 "net/http"
 "net/http/httptest"
 "testing"

 "github.com/hashicorp/terraform-plugin-framework/datasource"
 "github.com/hashicorp/terraform-plugin-framework/provider"
 "github.com/hashicorp/terraform-plugin-framework/tfsdk"
 "github.com/hashicorp/terraform-plugin-framework/types"
 sdk "example.com/lifecycle-sdk"
)

func TestAnonymousDataOnlyProviderUsesNewPinnedSDKOperation(t *testing.T) {
 calls:=0
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request) {
  calls++
  body,_:=io.ReadAll(r.Body)
  if r.Method!="GET" || r.RequestURI!="/v1/records/a%2Fb%20%E9%9B%AA" || r.Header.Get("Authorization")!="" || len(body)!=0 { t.Errorf("anonymous SDK wire: %s %s %v %s",r.Method,r.RequestURI,r.Header,body) }
  w.Header().Set("Content-Type","application/json")
  _,_=io.WriteString(w,`{"SetExtra":"read-only","credential-fingerprint":"sensitive","description":null,"enabled":false,"id":"a/b 雪","region":"east"}`)
 })); defer server.Close()
 p:=NewWithTransport("0.1.0",server.Client())()
 if len(p.Resources(context.Background()))!=0 { t.Fatal("data-only target invented resources") }
 var schema provider.SchemaResponse
 p.Schema(context.Background(),provider.SchemaRequest{},&schema)
 if len(schema.Schema.Attributes)!=1 { t.Fatal("anonymous provider invented credentials") }
 config:=tfsdk.State{Schema:schema.Schema}
 value:=struct{Endpoint types.String `tfsdk:"endpoint"`}{types.StringValue(server.URL+"/v1")}
 if d:=config.Set(context.Background(),value); d.HasError() { t.Fatal(d) }
 var configured provider.ConfigureResponse
 p.Configure(context.Background(),provider.ConfigureRequest{Config:tfsdk.Config{Schema:schema.Schema,Raw:config.Raw}},&configured)
 if configured.Diagnostics.HasError() { t.Fatal(configured.Diagnostics) }
 defer configured.DataSourceData.(*sdk.Client).CloseIdleConnections()
 d:=p.DataSources(context.Background())[0]()
 var dc datasource.ConfigureResponse
 d.(datasource.DataSourceWithConfigure).Configure(context.Background(),datasource.ConfigureRequest{ProviderData:configured.DataSourceData},&dc)
 if dc.Diagnostics.HasError() { t.Fatal(dc.Diagnostics) }
 var ds datasource.SchemaResponse
 d.Schema(context.Background(),datasource.SchemaRequest{},&ds)
 type model struct {
  ID types.String `tfsdk:"id"`
  Name types.String `tfsdk:"name"`
  Description types.String `tfsdk:"description"`
  Memo types.String `tfsdk:"memo"`
  Region types.String `tfsdk:"region"`
  Enabled types.Bool `tfsdk:"enabled"`
  Fingerprint types.String `tfsdk:"fingerprint"`
 }
 raw:=tfsdk.State{Schema:ds.Schema}
 if diagnostics:=raw.Set(context.Background(),model{ID:types.StringValue("a/b 雪")}); diagnostics.HasError() { t.Fatal(diagnostics) }
 response:=datasource.ReadResponse{State:tfsdk.State{Schema:ds.Schema}}
 d.Read(context.Background(),datasource.ReadRequest{Config:tfsdk.Config{Schema:ds.Schema,Raw:raw.Raw}},&response)
 if response.Diagnostics.HasError() { t.Fatal(response.Diagnostics) }
 var result model
 if diagnostics:=response.State.Get(context.Background(),&result); diagnostics.HasError() { t.Fatal(diagnostics) }
 if calls!=1 || result.Name.ValueString()!="read-only" || !result.Description.IsNull() || result.Enabled.IsUnknown() || result.Enabled.ValueBool() { t.Fatal("SDK result/state mapping changed") }
}
