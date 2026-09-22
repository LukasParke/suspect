package provider

import (
 "context"
 "io"
 "net/http"
 "net/http/httptest"
 "strings"
 "testing"

 "github.com/hashicorp/terraform-plugin-framework/resource"
 "github.com/hashicorp/terraform-plugin-framework/resource/schema"
 "github.com/hashicorp/terraform-plugin-framework/tfsdk"
 "github.com/hashicorp/terraform-plugin-framework/types"
)

func TestOptionalComputedComesFromSDKResponseWithoutInventingInput(t *testing.T) {
 server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
  body,_ := io.ReadAll(r.Body)
  if strings.Contains(string(body), `"memo"`) { t.Errorf("unconfigured computed value became an API input: %s", body) }
  want:=`{"SetExtra":"alpha","description":null,"enabled":false,"region":"east"}`
  if r.Method=="PUT" { want=`{"SetExtra":"alpha","description":null,"enabled":false}` }
  if string(body)!=want { t.Errorf("SDK wire changed: %s",body) }
  w.Header().Set("Content-Type","application/json")
  if r.Method=="POST" { w.WriteHeader(201) }
  _,_=io.WriteString(w, strings.Replace(record, `"region":"east"`, `"region":"east","memo":"remote-computed"`, 1))
 })); defer server.Close()
 r:=clientResource(t,server.Client(),server.URL+"/v1")
 var s resource.SchemaResponse
 r.Schema(context.Background(),resource.SchemaRequest{},&s)
 memo:=s.Schema.Attributes["memo"].(schema.StringAttribute)
 if !memo.Optional || !memo.Computed { t.Fatal("actual Framework schema lost optional-computed") }
 desired:=configured(); desired.Memo=types.StringUnknown()
 config:=configured()
 p,_,state:=requestData(t,r,desired); _,c,_:=requestData(t,r,config)
 response:=resource.CreateResponse{State:tfsdk.State{Schema:state.Schema}}
 r.Create(context.Background(),resource.CreateRequest{Plan:p,Config:c},&response)
 if response.Diagnostics.HasError() { t.Fatal(response.Diagnostics) }
 var created fixtureModel
 if d:=response.State.Get(context.Background(),&created); d.HasError() { t.Fatal(d) }
 if created.Memo.ValueString()!="remote-computed" || created.Memo.IsUnknown() { t.Fatal("response-computed value was lost") }
 desired=created; desired.Memo=types.StringUnknown(); config=created; config.Memo=types.StringNull()
 p,_,_=requestData(t,r,desired); _,c,_=requestData(t,r,config)
 updated:=resource.UpdateResponse{State:response.State}
 r.Update(context.Background(),resource.UpdateRequest{State:response.State,Plan:p,Config:c},&updated)
 if updated.Diagnostics.HasError() { t.Fatal(updated.Diagnostics) }
 var result fixtureModel
 if d:=updated.State.Get(context.Background(),&result); d.HasError() { t.Fatal(d) }
 if result.Memo.ValueString()!="remote-computed" { t.Fatal("update invented or dropped server-computed data") }
}
