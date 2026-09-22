//! Independent consumer acceptance for the installed m2-canonical-sdk package.
//! The recording transport is hand-authored from the canonical fixture, not
//! from generated sources; no request is assembled manually and no unsafe is
//! used anywhere.
/// Required inputs, presence and source tags are enforced by Rust types.
/// ```compile_fail
/// m2_canonical_sdk::operations::create_widget::CreateWidget::new();
/// ```
/// ```compile_fail
/// let _ = m2_canonical_sdk::models::__CREATE_BODY__::new(None);
/// ```
/// ```compile_fail
/// use m2_canonical_sdk::models::{WidgetPayload, SecurePayload};
/// let _ = WidgetPayload::StandardPayload(SecurePayload::new("vault".into()));
/// ```
pub struct NativeTypeContract;

#[cfg(test)]
mod tests {

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use m2_canonical_sdk::{
    Client, Credentials, Presence,
    http::{BoxError, Request, ResponseBody, SdkErrorKind, Transport, TransportResponse},
    models::{WidgetNode, WidgetPayload, __CREATE_BODY__ as CreateBody, __UPDATE_BODY__ as UpdateBody},
    operations::create_widget::{CreateWidget, CreateWidgetError, CreateWidgetSuccess, CreateWidgetApiError},
    operations::get_widget::{GetWidget, GetWidgetError, GetWidgetSuccess},
    operations::list_widgets::{ListWidgets, ListWidgetsSuccess},
    operations::update_widget::{UpdateWidget, UpdateWidgetSuccess},
};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

struct Body(Option<Vec<u8>>);
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        Ok(self.0.take())
    }
}

/// Hand-authored recording transport: exact request bytes in, one fixture out.
struct Recording {
    method: &'static str,
    url: &'static str,
    body: &'static str,
    response: &'static str,
    status: u16,
    calls: Arc<AtomicUsize>,
}

impl Transport for Recording {
    type Body = Body;
    async fn send(&self, request: Request) -> Result<TransportResponse<Body>, BoxError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.method, self.method);
        assert_eq!(request.url, self.url);
        assert!(request.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("authorization") && v == b"Bearer m2-key"));
        assert!(request.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("accept") && v == b"application/json"));
        match (&request.body, self.body) {
            (Some(body), expected) => assert_eq!(String::from_utf8_lossy(body), expected),
            (None, "") => {}
            other => panic!("unexpected body {other:?}"),
        }
        Ok(TransportResponse {
            status: self.status,
            headers: vec![("Content-Type".into(), b"Application/JSON; charset=utf-8".to_vec())],
            body: Body(Some(self.response.as_bytes().to_vec())),
        })
    }
}

fn make_client(recording: Recording) -> Client<Recording> {
    Client::with_transport(recording, Credentials::api_key("m2-key"))
}

const WIDGET: &str = r#"{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}"#;
const WIDGET_UPDATED_META: &str = r#"{"id":"w1","amount":0.0000000000000000001,"meta":"present","payload":{"kind":"standard","text":"plain"}}"#;
const LIST: &str = r#"{"items":[{"id":"w2","amount":0.0000000000000000001,"payload":{"kind":"secure","vault":"vlt-1"}}]}"#;

#[test]
fn create_preserves_exact_values_tagged_variants_recursion_and_null() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = make_client(Recording {
        method: "POST",
        url: "https://m2.example.test/api/v1/widgets",
        body: r#"{"name":"alpha"}"#,
        response: WIDGET,
        status: 200,
        calls: calls.clone(),
    });
    let CreateWidgetSuccess::Status200(widget) =
        block_on(client.create_widget(CreateWidget::new(CreateBody::new("alpha".into())))).unwrap();
    assert_eq!(widget.data.amount.as_str(), "9007199254740993.000000000000000001");
    assert!(matches!(widget.data.meta, Presence::Null));
    let child: &WidgetNode = widget.data.child.as_ref().expect("declared child");
    assert_eq!(child.label, "root");
    assert!(child.child.is_none());
    match widget.data.payload {
        WidgetPayload::StandardPayload(payload) => assert_eq!(payload.text, "plain"),
        other => panic!("wrong source-tagged variant: {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn list_get_and_update_preserve_form_queries_paths_and_nullable_presence() {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = make_client(Recording {
        method: "GET",
        url: "https://m2.example.test/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2",
        body: "",
        response: LIST,
        status: 200,
        calls: calls.clone(),
    });
    let ListWidgetsSuccess::Status200(list) = block_on(client.list_widgets(
        ListWidgets::new()
            .with_tag("a".into())
            .with_tags(vec!["x".into(), "y".into()])
            .with_labels(vec!["a,b".into(), "c".into()])
            .with_limit(2),
    ))
    .unwrap();
    let first = &list.data.items[0];
    assert_eq!(first.amount.as_str(), "0.0000000000000000001");
    match &first.payload {
        WidgetPayload::SecurePayload(payload) => assert_eq!(payload.vault, "vlt-1"),
        other => panic!("wrong source-tagged variant: {other:?}"),
    }

    assert!(matches!(first.meta,Presence::Absent));
    let read = make_client(Recording {method:"GET",url:"https://m2.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A",body:"",response:WIDGET,status:200,calls:calls.clone()});
    let GetWidgetSuccess::Status200(widget)=block_on(read.get_widget(GetWidget::new("a/b 雪!'()*".into()))).unwrap();
    assert_eq!(widget.data.id,"w1");
    let updated = make_client(Recording {
        method: "PATCH",
        url: "https://m2.example.test/api/v1/widgets/w1",
        body: "{}",
        response: WIDGET_UPDATED_META,
        status: 200,
        calls: calls.clone(),
    });
    let UpdateWidgetSuccess::Status200(widget) =
        block_on(updated.update_widget(UpdateWidget::new("w1".into(), UpdateBody::new()))).unwrap();
    assert_eq!(widget.data.amount.as_str(), "0.0000000000000000001");
    assert!(matches!(&widget.data.meta, Presence::Value(value) if value == "present"));
    assert_eq!(calls.load(Ordering::SeqCst),3);
}
#[test]
fn invalid_inputs_never_enter_transport_and_failures_stay_typed() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut body=CreateBody::new("alpha".into());
    body.name.clear();
    {
        let client = make_client(Recording {
            method: "POST",
            url: "https://m2.example.test/api/v1/widgets",
            body: r#"{"name":"alpha"}"#,
            response: WIDGET,
            status: 200,
            calls: calls.clone(),
        });
        match block_on(client.create_widget(CreateWidget::new(body))).unwrap_err() {
            CreateWidgetError::Sdk(error) => assert_eq!(error.kind, SdkErrorKind::RequestValidation),
            other => panic!("wrong failure: {other:?}"),
        }
    }
    let bad_path=make_client(Recording {method:"GET",url:"",body:"",response:WIDGET,status:200,calls:calls.clone()});
    match block_on(bad_path.get_widget(GetWidget::new("..".into()))).unwrap_err() {
        GetWidgetError::Sdk(error)=>assert_eq!(error.kind,SdkErrorKind::RequestRepresentation),
        other=>panic!("wrong failure {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // Malformed declared success bodies and undeclared statuses stay typed.
    let decoder = make_client(Recording {
        method: "POST",
        url: "https://m2.example.test/api/v1/widgets",
        body: r#"{"name":"alpha"}"#,
        response: r#"{"id":1}"#,
        status: 200,
        calls: calls.clone(),
    });
    match block_on(decoder.create_widget(CreateWidget::new(CreateBody::new("alpha".into())))).unwrap_err() {
        CreateWidgetError::Sdk(error) => {
            assert_eq!(error.kind, SdkErrorKind::ResponseDecoding);
            assert_eq!(error.status, Some(200));
        }
        other => panic!("wrong failure: {other:?}"),
    }
    let undeclared = make_client(Recording {
        method: "POST",
        url: "https://m2.example.test/api/v1/widgets",
        body: r#"{"name":"alpha"}"#,
        response: "undeclared",
        status: 500,
        calls: calls.clone(),
    });
    match block_on(undeclared.create_widget(CreateWidget::new(CreateBody::new("alpha".into())))).unwrap_err() {
        CreateWidgetError::Sdk(error) => assert_eq!(error.kind, SdkErrorKind::UnexpectedResponse),
        other => panic!("wrong failure: {other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let rejected=make_client(Recording {method:"POST",url:"https://m2.example.test/api/v1/widgets",body:r#"{"name":"alpha"}"#,response:r#"{"message":"rejected"}"#,status:422,calls:calls.clone()});
    match block_on(rejected.create_widget(CreateWidget::new(CreateBody::new("alpha".into())))).unwrap_err() {
        CreateWidgetError::Api(error)=>match *error {
            CreateWidgetApiError::Status422(response)=>{assert_eq!(response.status,422);assert_eq!(response.data.message,"rejected");},
            other=>panic!("wrong API error {other:?}"),
        },
        other=>panic!("wrong error {other:?}"),
    }
}
}
