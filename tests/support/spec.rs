//! Tools driven by the locally downloaded OpenAPI document: an example generator, a
//! schema validator and a mock server that validates every request against
//! the specification and answers with a schema-derived example.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use percent_encoding::percent_decode_str;
use regex::Regex;
use serde_json::{Map, Value, json};

const HTTP_METHODS: [&str; 5] = ["get", "post", "put", "delete", "patch"];

/// The locally downloaded OpenAPI document.
pub struct Spec {
    pub doc: Value,
}

/// One operation of the specification.
#[derive(Debug, Clone)]
pub struct Operation {
    pub method: String,
    pub path: String,
    pub id: String,
}

impl Spec {
    pub fn load() -> Spec {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/spec/openapi.json");
        let text = std::fs::read_to_string(path).unwrap_or_else(|err| {
            panic!(
                "Cannot read {path}: {err}. Download the OpenAPI specification from \
                 https://docs.ironbeamapi.com/ and save it as spec/openapi.json; \
                 see spec/README.md for setup instructions."
            )
        });
        Spec {
            doc: serde_json::from_str(&text).expect("spec/openapi.json must contain valid JSON"),
        }
    }

    pub fn operations(&self) -> Vec<Operation> {
        let mut out = Vec::new();
        for (path, item) in self.doc["paths"].as_object().unwrap() {
            for method in HTTP_METHODS {
                if let Some(op) = item.get(method) {
                    out.push(Operation {
                        method: method.to_uppercase(),
                        path: path.clone(),
                        id: op["operationId"].as_str().unwrap_or_default().to_owned(),
                    });
                }
            }
        }
        out
    }

    pub fn schema(&self, name: &str) -> &Value {
        let schema = &self.doc["components"]["schemas"][name];
        assert!(!schema.is_null(), "schema {name} exists");
        schema
    }

    /// Follows `$ref` chains.
    pub fn resolve<'a>(&'a self, mut node: &'a Value) -> &'a Value {
        while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let pointer = reference.trim_start_matches('#');
            node = self
                .doc
                .pointer(pointer)
                .unwrap_or_else(|| panic!("dangling $ref {reference}"));
        }
        node
    }

    /// Flattens `allOf` into a single list of `(properties, required)`.
    fn object_parts<'a>(
        &'a self,
        schema: &'a Value,
        props: &mut Vec<(&'a String, &'a Value)>,
        required: &mut Vec<&'a str>,
    ) {
        let schema = self.resolve(schema);
        if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
            for part in all {
                self.object_parts(part, props, required);
            }
        }
        if let Some(p) = schema.get("properties").and_then(Value::as_object) {
            props.extend(p.iter());
        }
        if let Some(r) = schema.get("required").and_then(Value::as_array) {
            required.extend(r.iter().filter_map(Value::as_str));
        }
    }

    fn is_object(&self, schema: &Value) -> bool {
        let schema = self.resolve(schema);
        schema.get("properties").is_some()
            || schema.get("allOf").is_some()
            || schema.get("type").and_then(Value::as_str) == Some("object")
    }

    /// Builds a value conforming to `schema`, with every property populated.
    pub fn example(&self, schema: &Value) -> Value {
        self.example_at(schema, 0)
    }

    fn example_at(&self, schema: &Value, depth: usize) -> Value {
        assert!(depth < 32, "schema recursion too deep");
        let outer_example = schema.get("example");
        let schema = self.resolve(schema);
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            // First value that is not the empty-string "INVALID" marker.
            return values
                .iter()
                .find(|v| v.as_str() != Some(""))
                .or_else(|| values.first())
                .cloned()
                .unwrap_or(Value::Null);
        }
        if self.is_object(schema) {
            let (mut props, mut required) = (Vec::new(), Vec::new());
            self.object_parts(schema, &mut props, &mut required);
            let mut out = Map::new();
            for (name, prop) in props {
                out.insert(name.clone(), self.example_at(prop, depth + 1));
            }
            return Value::Object(out);
        }
        let example = outer_example.or_else(|| schema.get("example"));
        match schema.get("type").and_then(Value::as_str) {
            Some("array") => {
                let items = match schema.get("items") {
                    // The document uses tuple-style `items` in one place.
                    Some(Value::Array(list)) => list.first().cloned().unwrap_or(json!({})),
                    Some(items) => items.clone(),
                    None => json!({}),
                };
                json!([self.example_at(&items, depth + 1)])
            }
            Some("string") => match example {
                Some(Value::String(s)) if self.string_ok(schema, s) => json!(s),
                _ => match schema.get("format").and_then(Value::as_str) {
                    Some("date-time") => json!("2016-08-01T00:00:00Z"),
                    Some("uuid") => json!("123e4567-e89b-12d3-a456-426614174000"),
                    _ => json!(self.string_default(schema)),
                },
            },
            Some("integer") => match example {
                Some(v) if v.is_i64() || v.is_u64() => v.clone(),
                _ => json!(7),
            },
            Some("number") => match example {
                Some(v) if v.is_number() => v.clone(),
                // `number` with an integer format (`long`) means a whole number.
                _ if matches!(
                    schema.get("format").and_then(Value::as_str),
                    Some("long" | "int64" | "int32")
                ) =>
                {
                    json!(7)
                }
                _ => json!(1.5),
            },
            Some("boolean") => match example {
                Some(Value::Bool(b)) => json!(b),
                _ => json!(true),
            },
            other => panic!("cannot build an example for type {other:?}: {schema}"),
        }
    }

    fn string_ok(&self, schema: &Value, s: &str) -> bool {
        let max = schema
            .get("maxLength")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        let pattern_ok = schema
            .get("pattern")
            .and_then(Value::as_str)
            .is_none_or(|p| Regex::new(p).map(|re| re.is_match(s)).unwrap_or(true));
        (s.chars().count() as u64) <= max && pattern_ok
    }

    fn string_default(&self, schema: &Value) -> String {
        for candidate in [
            "XCME:ES.Z26",
            "20260102",
            "12345.5",
            "USD",
            "X",
            "1-1",
            "text",
        ] {
            if self.string_ok(schema, candidate) {
                return candidate.to_owned();
            }
        }
        "text".to_owned()
    }

    /// Validates `value` against `schema`, appending problems to `problems`.
    ///
    /// Checks types, enums, required and unknown properties, numeric bounds,
    /// string length and pattern, and array length.
    pub fn validate(&self, schema: &Value, value: &Value, at: &str, problems: &mut Vec<String>) {
        let schema = self.resolve(schema);
        if value.is_null() {
            if schema.get("nullable") != Some(&Value::Bool(true)) {
                problems.push(format!("{at}: null is not allowed"));
            }
            return;
        }
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            if !values.contains(value) {
                problems.push(format!("{at}: {value} is not one of {values:?}"));
            }
            return;
        }
        if self.is_object(schema) {
            let Some(object) = value.as_object() else {
                problems.push(format!("{at}: expected an object, got {value}"));
                return;
            };
            let (mut props, mut required) = (Vec::new(), Vec::new());
            self.object_parts(schema, &mut props, &mut required);
            for name in required {
                // The document lists required names that are not declared
                // properties (OrderRequest: "account"); those cannot be honoured.
                let declared = props.iter().any(|(n, _)| n.as_str() == name);
                if declared && !object.contains_key(name) {
                    problems.push(format!("{at}: required property {name} is missing"));
                }
            }
            for (name, item) in object {
                match props.iter().find(|(n, _)| *n == name) {
                    Some((_, prop)) => self.validate(prop, item, &format!("{at}.{name}"), problems),
                    None => problems.push(format!("{at}: property {name} is not in the schema")),
                }
            }
            return;
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("array") => {
                let Some(items) = value.as_array() else {
                    problems.push(format!("{at}: expected an array, got {value}"));
                    return;
                };
                if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
                    if items.len() as u64 > max {
                        problems.push(format!("{at}: {} items exceed maxItems {max}", items.len()));
                    }
                }
                if let Some(item_schema) = schema.get("items").filter(|s| s.is_object()) {
                    for (i, item) in items.iter().enumerate() {
                        self.validate(item_schema, item, &format!("{at}[{i}]"), problems);
                    }
                }
            }
            Some("string") => match value.as_str() {
                None => problems.push(format!("{at}: expected a string, got {value}")),
                Some(s) => {
                    if !self.string_ok(schema, s) {
                        problems.push(format!("{at}: {s:?} violates length/pattern constraints"));
                    }
                }
            },
            Some("integer") => {
                if !(value.is_i64() || value.is_u64()) {
                    problems.push(format!("{at}: expected an integer, got {value}"));
                }
                self.check_bounds(schema, value, at, problems);
            }
            Some("number") => {
                if !value.is_number() {
                    problems.push(format!("{at}: expected a number, got {value}"));
                }
                self.check_bounds(schema, value, at, problems);
            }
            Some("boolean") => {
                if !value.is_boolean() {
                    problems.push(format!("{at}: expected a boolean, got {value}"));
                }
            }
            _ => {}
        }
    }

    fn check_bounds(&self, schema: &Value, value: &Value, at: &str, problems: &mut Vec<String>) {
        let Some(v) = value.as_f64() else { return };
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| v < min)
        {
            problems.push(format!("{at}: {v} is below the minimum"));
        }
        if schema
            .get("maximum")
            .and_then(Value::as_f64)
            .is_some_and(|max| v > max)
        {
            problems.push(format!("{at}: {v} is above the maximum"));
        }
    }

    /// Converts a path/query string into the JSON type its schema asks for.
    fn coerce(&self, schema: &Value, raw: &str) -> Value {
        let schema = self.resolve(schema);
        match schema.get("type").and_then(Value::as_str) {
            Some("integer") => raw.parse::<i64>().map(Value::from).unwrap_or(json!(raw)),
            Some("number") => raw.parse::<f64>().map(Value::from).unwrap_or(json!(raw)),
            Some("boolean") => raw.parse::<bool>().map(Value::from).unwrap_or(json!(raw)),
            // Arrays in the query are comma separated (see the `symbols` parameter).
            Some("array") => Value::Array(raw.split(',').map(Value::from).collect()),
            _ => json!(raw),
        }
    }
}

type PathParams = Vec<(String, String)>;

/// What the spec server saw for one request.
#[derive(Debug, Clone)]
pub struct Call {
    pub operation_id: String,
    pub problems: Vec<String>,
    pub response: Value,
}

/// A server that accepts exactly what the specification describes.
#[derive(Clone)]
pub struct SpecServer {
    pub addr: SocketAddr,
    spec: Arc<Spec>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl SpecServer {
    pub async fn start() -> SpecServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = SpecServer {
            addr,
            spec: Arc::new(Spec::load()),
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new().fallback(handle).with_state(server.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        server
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v2", self.addr)
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn last_call(&self) -> Call {
        self.calls
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("at least one call was made")
    }

    /// Finds the operation for a request: same method, same number of
    /// segments, literal segments equal; the most literal match wins.
    fn route(&self, method: &str, segments: &[String]) -> Option<(String, PathParams)> {
        let mut best: Option<(usize, String, PathParams)> = None;
        for (template, item) in self.spec.doc["paths"].as_object().unwrap() {
            if item.get(method).is_none() {
                continue;
            }
            let parts: Vec<&str> = template.split('/').filter(|p| !p.is_empty()).collect();
            if parts.len() != segments.len() {
                continue;
            }
            let mut params = Vec::new();
            let mut literals = 0;
            let mut matched = true;
            for (part, segment) in parts.iter().zip(segments) {
                match part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                    Some(name) => params.push((name.to_owned(), segment.clone())),
                    None if part == segment => literals += 1,
                    None => {
                        matched = false;
                        break;
                    }
                }
            }
            if matched && best.as_ref().is_none_or(|(score, _, _)| literals > *score) {
                best = Some((literals, template.clone(), params));
            }
        }
        best.map(|(_, template, params)| (template, params))
    }
}

async fn handle(State(server): State<SpecServer>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    let spec = &server.spec;
    let method = parts.method.as_str().to_lowercase();
    let mut problems = Vec::new();

    let path = parts.uri.path();
    let Some(rest) = path.strip_prefix("/v2/") else {
        return (StatusCode::NOT_FOUND, "missing /v2 prefix").into_response();
    };
    let segments: Vec<String> = rest
        .split('/')
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect();
    let Some((template, path_params)) = server.route(&method, &segments) else {
        let call = Call {
            operation_id: String::new(),
            problems: vec![format!("no operation for {method} {path}")],
            response: Value::Null,
        };
        server.calls.lock().unwrap().push(call);
        return (StatusCode::NOT_FOUND, "no such operation").into_response();
    };
    let op = &spec.doc["paths"][&template][&method];
    let operation_id = op["operationId"].as_str().unwrap_or_default().to_owned();

    // Authentication.
    let secured = op
        .get("security")
        .and_then(Value::as_array)
        .is_some_and(|s| !s.is_empty());
    let bearer = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer ") && v.len() > 7);
    if secured && !bearer {
        problems.push("secured operation called without a bearer token".into());
    }

    // Parameters.
    let query: Vec<(String, String)> =
        url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
    let declared: Vec<&Value> = op
        .get("parameters")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(|p| spec.resolve(p)).collect())
        .unwrap_or_default();
    for param in &declared {
        let name = param["name"].as_str().unwrap();
        let location = param["in"].as_str().unwrap();
        let raw = match location {
            "path" => path_params
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            "query" => query
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str()),
            _ => None,
        };
        match raw {
            Some(raw) => {
                let value = spec.coerce(&param["schema"], raw);
                spec.validate(
                    &param["schema"],
                    &value,
                    &format!("{location} parameter {name}"),
                    &mut problems,
                );
            }
            None if param["required"] == true => {
                problems.push(format!("required {location} parameter {name} is missing"));
            }
            None => {}
        }
    }
    for (name, _) in &query {
        let known = declared
            .iter()
            .any(|p| p["in"] == "query" && p["name"] == name.as_str());
        if !known {
            problems.push(format!(
                "query parameter {name} is not in the specification"
            ));
        }
    }

    // Body.
    let body_schema = op.pointer("/requestBody/content/application~1json/schema");
    match (body_schema, bytes.is_empty()) {
        (Some(schema), false) => {
            let content_type = parts
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !content_type.starts_with("application/json") {
                problems.push(format!(
                    "body sent as {content_type:?}, expected application/json"
                ));
            }
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => spec.validate(schema, &value, "body", &mut problems),
                Err(e) => problems.push(format!("body is not JSON: {e}")),
            }
        }
        (Some(_), true) => problems.push("request body is missing".into()),
        (None, false) => problems.push("unexpected request body".into()),
        (None, true) => {}
    }

    // Response.
    let response = op
        .pointer("/responses/200/content/application~1json/schema")
        .map(|schema| spec.example(schema))
        .unwrap_or(json!({}));
    server.calls.lock().unwrap().push(Call {
        operation_id,
        problems,
        response: response.clone(),
    });
    let _ = Body::empty();
    Json(response).into_response()
}

/// Every key path present in `expected` must be present in `actual`
/// (values are not compared). Returns the missing paths.
pub fn missing_keys(expected: &Value, actual: &Value, at: &str, out: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (key, value) in e {
                match a.get(key) {
                    Some(found) if !found.is_null() || value.is_null() => {
                        missing_keys(value, found, &format!("{at}.{key}"), out)
                    }
                    _ => out.push(format!("{at}.{key}")),
                }
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if !e.is_empty() && a.is_empty() {
                out.push(format!("{at}[]"));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                missing_keys(ev, av, &format!("{at}[{i}]"), out);
            }
        }
        (Value::Object(_) | Value::Array(_), _) => out.push(at.to_owned()),
        _ => {}
    }
}
