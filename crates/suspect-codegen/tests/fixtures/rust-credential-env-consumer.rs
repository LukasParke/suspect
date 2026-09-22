use sdk::{Client,Credentials,http::{BoxError,Request,ResponseBody,Transport,TransportResponse,SdkErrorKind}};
use std::sync::{Arc,Mutex};
#[derive(Clone,Default)] struct Capture(Arc<Mutex<Vec<Request>>>);
struct Empty;
impl ResponseBody for Empty {async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError>{Ok(None)}}
impl Transport for Capture {
    type Body=Empty;
    async fn send(&self,request:Request)->Result<TransportResponse<Empty>,BoxError>{
        assert_eq!(request.method,"GET");assert!(request.url.starts_with("https://api.example.test/v1/"));
        self.0.lock().unwrap().push(request);Ok(TransportResponse{status:204,headers:vec![],body:Empty})
    }
}
fn header<'a>(request:&'a Request,name:&str)->Option<&'a str>{request.headers.iter().find(|(key,_)|key.eq_ignore_ascii_case(name)).map(|(_,value)|std::str::from_utf8(value).unwrap())}
fn missing(error:sdk::operations::get_current_key::GetCurrentKeyError){
    let sdk::operations::get_current_key::GetCurrentKeyError::Sdk(error)=error else{panic!("local auth failure")};
    assert_eq!(error.kind,SdkErrorKind::RequestValidation);assert!(error.status.is_none());
    assert!(!format!("{error:?} {error}").contains("secret"));assert!(!format!("{error:?} {error}").contains("early-token"));
}
fn main(){
    let mode=std::env::args().nth(1).unwrap();let record=Capture::default();
    let before=Client::with_transport_from_env(record.clone());
    // Both snapshots precede every executor/transport worker. This standalone
    // process has one thread during these environment writes.
    if mode=="snapshot"{std::env::set_var("SDK_TEST_BEARER","later-token");}
    let after=Client::with_transport_from_env(record.clone());
    if mode=="snapshot"{std::env::remove_var("SDK_TEST_BEARER");}
    let unavailable=Client::with_transport_from_env(record.clone());
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async{
        // Requested helper, actual signature and feature compile. No requests
        // use this network transport; all HTTP below uses Capture.
        let _:Client<sdk::reqwest_transport::ReqwestTransport>=Client::from_env().unwrap();
        let _:Client<sdk::reqwest_transport::ReqwestTransport>=Client::with_reqwest(Credentials::new()).unwrap();
        match mode.as_str(){
            "positive"=>{
                before.get_current_key_default().await.unwrap();before.either_default().await.unwrap();before.both_default().await.unwrap();
                before.query_key_default().await.unwrap();before.cookie_key_default().await.unwrap();
                before.__FROM_ENV_OP__().await.unwrap();before.__TRANSPORT_ENV_OP__().await.unwrap();
                let records=record.0.lock().unwrap();assert_eq!(records[0].url,"https://api.example.test/v1/key");
                assert_eq!(header(&records[0],"authorization"),Some("Bearer early-token"));
                assert_eq!(header(&records[1],"authorization"),Some("Bearer early-token"));assert_eq!(header(&records[1],"x-key"),None);
                assert_eq!(header(&records[2],"authorization"),Some("Bearer early-token"));assert_eq!(header(&records[2],"x-key"),Some("header-key"));
                assert_eq!(records[3].url,"https://api.example.test/v1/query?key=query%20%26%2B");assert_eq!(header(&records[4],"cookie"),Some("sid=cookie-key"));drop(records);
                let explicit=Client::with_transport(record.clone(),Credentials::api_key("explicit-token"));
                explicit.get_current_key_default().await.unwrap();assert_eq!(header(record.0.lock().unwrap().last().unwrap(),"authorization"),Some("Bearer explicit-token"));
                let count=record.0.lock().unwrap().len();assert!(explicit.both_default().await.is_err());assert_eq!(record.0.lock().unwrap().len(),count);
                missing(Client::with_transport(record.clone(),Credentials::new()).get_current_key_default().await.unwrap_err());
                missing(Client::with_transport(record.clone(),Credentials::api_key("")).get_current_key_default().await.unwrap_err());
                let partial=Client::with_transport(record.clone(),Credentials::key_header("explicit-header"));
                partial.either_default().await.unwrap();assert_eq!(header(record.0.lock().unwrap().last().unwrap(),"authorization"),None);
                assert!(partial.both_default().await.is_err());
                let chosen=Client::with_transport(record.clone(),Credentials::from_env().select_alternative(1));
                chosen.either_default().await.unwrap();assert_eq!(header(record.0.lock().unwrap().last().unwrap(),"authorization"),None);
                assert_eq!(header(record.0.lock().unwrap().last().unwrap(),"x-key"),Some("header-key"));
                Client::with_transport(record.clone(),Credentials::__FROM_ENV_CTOR__("explicit-other")).__FROM_ENV_OP__().await.unwrap();
                before.anonymous_default().await.unwrap();assert_eq!(header(record.0.lock().unwrap().last().unwrap(),"authorization"),None);
            },
            "snapshot"=>{
                before.get_current_key_default().await.unwrap();after.get_current_key_default().await.unwrap();
                missing(unavailable.get_current_key_default().await.unwrap_err());
                let records=record.0.lock().unwrap();assert_eq!(records.len(),2);
                assert_eq!(header(&records[0],"authorization"),Some("Bearer early-token"));assert_eq!(header(&records[1],"authorization"),Some("Bearer later-token"));
            },
            "alternate"=>{
                before.either_default().await.unwrap();missing(before.get_current_key_default().await.unwrap_err());assert!(before.both_default().await.is_err());
                let records=record.0.lock().unwrap();assert_eq!(records.len(),1);assert_eq!(header(&records[0],"authorization"),None);assert_eq!(header(&records[0],"x-key"),Some("header-key"));
            },
            "missing"|"empty"|"unavailable"=>{
                missing(before.get_current_key_default().await.unwrap_err());assert!(before.either_default().await.is_err());assert!(before.both_default().await.is_err());
                assert!(record.0.lock().unwrap().is_empty());before.anonymous_default().await.unwrap();before.optional_default().await.unwrap();
                let chosen=Client::with_transport(record.clone(),Credentials::from_env().select_alternative(0));assert!(chosen.optional_default().await.is_err());
                assert_eq!(record.0.lock().unwrap().len(),2);
                assert!(record.0.lock().unwrap().iter().all(|r|header(r,"authorization").is_none()));
            },
            "invalid"=>{missing(before.get_current_key_default().await.unwrap_err());assert!(record.0.lock().unwrap().is_empty());before.anonymous_default().await.unwrap();},
            _=>panic!("unknown test case"),
        }
    });
}
