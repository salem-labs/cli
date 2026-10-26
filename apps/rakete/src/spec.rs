//! Die Beschreibung des Servers, gelesen als Liste von Vorgängen.
//!
//! **Hier steht keine einzige Zeile über Kunden, Einsätze oder Belege**,
//! und das ist die Entscheidung, an der alles hängt. Rakete hat
//! zweihundertvierundneunzig Vorgänge; die von Hand als Unterbefehle zu
//! schreiben wäre einmal viel Arbeit und danach für immer eine Liste,
//! die hinterherhinkt. Der nächste Endpunkt käme dazu, niemand trüge ihn
//! nach, und das Werkzeug wäre still unvollständig.
//!
//! Gelesen wird **zur Laufzeit** und nicht beim Bauen: zwei Betriebe
//! können verschiedene Fassungen von Rakete laufen haben, und dann soll
//! jeder die Befehle bekommen, die sein Server auch kennt.

use std::collections::BTreeMap;

use l3pus_cli::exit::{Failure, Outcome};

/// Ein einzelner Aufruf, so wie die Kommandozeile ihn anbietet.
#[derive(Debug, Clone)]
pub struct Operation {
    /// Die Gruppe, unter der er steht: `customers`, `dispatch`, …
    pub group: String,
    /// Der Befehl innerhalb der Gruppe: `list`, `get`, `create`, …
    pub name: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    /// The whole description the server wrote, not only its first line.
    ///
    /// **This is where the server says how a step fits into a flow**: what
    /// has to exist before, what comes after, what cannot be undone. The
    /// summary is a list entry for `--help`; an agent deciding what to do
    /// next needs the rest.
    pub description: String,
    /// Was im Pfad steht und deshalb angegeben werden **muss**.
    pub path_params: Vec<Param>,
    /// Was hinten an die Adresse gehängt wird.
    pub query_params: Vec<Param>,
    /// Ob der Vorgang einen Rumpf erwartet.
    pub has_body: bool,
    /// Ob der Rumpf rohe Bytes sind statt JSON.
    ///
    /// **Das Hochladen eines Papiers ist der Fall** (ADR-038): dort geht
    /// eine Datei hinaus, kein Feldsatz. Ohne diese Unterscheidung böte
    /// die Kommandozeile den Befehl an und könnte ihn nicht ausführen,
    /// und das ist schlimmer, als ihn nicht anzubieten.
    pub wants_file: bool,
    /// The content type the server declares for a file body, such as
    /// `image/png` for a logo. Empty where it takes any bytes.
    pub file_type: String,
    /// Welche Felder der Rumpf mindestens braucht. Steht in der
    /// Fehlermeldung, damit niemand die Beschreibung lesen muss.
    pub required_fields: Vec<String>,
    /// The JSON body as a schema with every reference resolved.
    ///
    /// **Required fields alone are not enough to write a body.** A theme
    /// has six nested groups with fixed sets of values, and the server
    /// refuses unknown fields; a caller that only knows the top-level name
    /// `spec` can only guess, and every guess is refused. The server's
    /// description holds all of it, so it is passed on instead of being
    /// boiled down.
    pub body_schema: Option<serde_json::Value>,
    /// What a successful answer is, by content type: JSON, a PDF, an image.
    pub returns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub required: bool,
    /// The first line, for `--help`.
    pub description: String,
    /// All of it, for an agent.
    pub full_description: String,
    /// The value's schema, references resolved: type, allowed values.
    pub schema: Option<serde_json::Value>,
}

/// Alle Vorgänge eines Servers, nach Gruppe und Befehl.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub groups: BTreeMap<String, BTreeMap<String, Operation>>,
}

impl Catalog {
    pub fn find(&self, group: &str, name: &str) -> Option<&Operation> {
        self.groups.get(group)?.get(name)
    }

    pub fn count(&self) -> usize {
        self.groups.values().map(BTreeMap::len).sum()
    }
}

impl Operation {
    /// Everything there is to know before calling this, as one document.
    ///
    /// **One shape for the terminal and for MCP**, so `rakete describe`
    /// and `rakete_describe` cannot drift apart: both print this.
    pub fn detail(&self) -> serde_json::Value {
        let param = |param: &Param| {
            serde_json::json!({
                "name": param.name,
                "required": param.required,
                "description": param.full_description,
                "schema": param.schema,
            })
        };
        serde_json::json!({
            "group": self.group,
            "command": self.name,
            "method": self.method,
            "path": self.path,
            "description": self.description,
            "path_values": self.path_params.iter().map(param).collect::<Vec<_>>(),
            "query_values": self.query_params.iter().map(param).collect::<Vec<_>>(),
            "body": if self.wants_file {
                serde_json::json!({
                    "file": true,
                    "content_type": if self.file_type.is_empty() { "beliebig" } else { self.file_type.as_str() },
                })
            } else if self.has_body {
                serde_json::json!({
                    "required_fields": self.required_fields,
                    "schema": self.body_schema,
                })
            } else {
                serde_json::Value::Null
            },
            "returns": self.returns,
        })
    }
}

/// Aus einer OpenAPI-Beschreibung die Befehle bauen.
pub fn parse(document: &serde_json::Value) -> Outcome<Catalog> {
    let paths = document
        .get("paths")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            Failure::unreachable("Die Beschreibung des Servers hat keine Wege. Ist das Rakete?")
        })?;

    let mut catalog = Catalog::default();
    // Wo ein Name zweimal fiele, bekommen beide den vollen: lieber
    // `list-customers` als zwei Befehle, von denen einer den anderen
    // verdeckt.
    let mut collisions: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut entries: Vec<Operation> = Vec::new();

    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for (method, operation) in item {
            let upper = method.to_uppercase();
            if !matches!(upper.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
                continue;
            }
            let group = group_of(operation);
            let id = operation
                .get("operationId")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let name = command_name(&id, &group);
            *collisions.entry((group.clone(), name.clone())).or_default() += 1;
            entries.push(Operation {
                group,
                name,
                method: upper,
                path: path.clone(),
                summary: summary_of(operation),
                description: description_of(operation),
                path_params: params_of(document, operation, "path"),
                query_params: params_of(document, operation, "query"),
                has_body: operation.get("requestBody").is_some(),
                wants_file: wants_file(operation),
                file_type: file_type(operation),
                required_fields: required_fields(document, operation),
                body_schema: body_schema(document, operation),
                returns: returns(operation),
            });
        }
    }

    for mut operation in entries {
        let key = (operation.group.clone(), operation.name.clone());
        if collisions.get(&key).copied().unwrap_or(0) > 1 {
            // Zwei Vorgänge derselben Gruppe wollten denselben Namen.
            // Dann bekommt jeder seinen vollen, statt dass einer
            // verschwindet.
            operation.name = full_name(&operation);
        }
        catalog
            .groups
            .entry(operation.group.clone())
            .or_default()
            .insert(operation.name.clone(), operation);
    }

    if catalog.count() == 0 {
        return Err(Failure::unreachable(
            "Die Beschreibung des Servers enthält keinen einzigen Vorgang.",
        ));
    }
    Ok(catalog)
}

/// Die Gruppe kommt aus dem Schildchen, ohne den Modulpfad davor.
///
/// utoipa schreibt `http::customers`; für einen Menschen auf der
/// Kommandozeile heißt das `customers`.
fn group_of(operation: &serde_json::Value) -> String {
    let tag = operation
        .get("tags")
        .and_then(|value| value.as_array())
        .and_then(|tags| tags.first())
        .and_then(|tag| tag.as_str())
        .unwrap_or("api");
    let short = tag.rsplit("::").next().unwrap_or(tag);
    kebab(short)
}

/// Aus `list_absences` in der Gruppe `absences` wird `list`.
///
/// **Die Wiederholung fällt weg und sonst nichts.** `rakete absences
/// list-absences` liest sich wie ein Formular; `rakete absences
/// list-calendars` dagegen sagt etwas, also bleibt es stehen.
fn command_name(id: &str, group: &str) -> String {
    let name = kebab(id);
    let group_singular = group.strip_suffix('s').unwrap_or(group);
    for suffix in [format!("-{group}"), format!("-{group_singular}")] {
        if let Some(rest) = name.strip_suffix(&suffix)
            && !rest.is_empty()
        {
            return rest.to_string();
        }
    }
    name
}

fn full_name(operation: &Operation) -> String {
    // Methode davor macht zwei gleichnamige Vorgänge unterscheidbar,
    // ohne dass man die Beschreibung lesen muss.
    format!("{}-{}", operation.method.to_lowercase(), operation.name)
}

fn kebab(text: &str) -> String {
    text.replace('_', "-").to_lowercase()
}

fn summary_of(operation: &serde_json::Value) -> String {
    for key in ["summary", "description"] {
        if let Some(text) = operation.get(key).and_then(|value| value.as_str())
            && !text.trim().is_empty()
        {
            // Nur die erste Zeile: `--help` ist eine Liste und kein
            // Fließtext.
            return text.lines().next().unwrap_or("").trim().to_string();
        }
    }
    String::new()
}

/// The description in full, summary first.
///
/// utoipa splits a doc comment into a one-line summary and the rest; put
/// back together it is the text the server's author wrote.
fn description_of(operation: &serde_json::Value) -> String {
    let parts: Vec<&str> = ["summary", "description"]
        .iter()
        .filter_map(|key| operation.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect();
    parts.join("\n\n")
}

fn params_of(
    document: &serde_json::Value,
    operation: &serde_json::Value,
    place: &str,
) -> Vec<Param> {
    operation
        .get("parameters")
        .and_then(|value| value.as_array())
        .map(|list| {
            list.iter()
                .filter(|param| param.get("in").and_then(|v| v.as_str()) == Some(place))
                .filter_map(|param| {
                    Some(Param {
                        name: param.get("name")?.as_str()?.to_string(),
                        required: param
                            .get("required")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(place == "path"),
                        description: param
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                        full_description: param
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                        schema: param
                            .get("schema")
                            .map(|schema| resolve(document, schema, &mut Vec::new())),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ob dieser Vorgang eine Datei will.
///
/// Erkannt an der Sorte des Rumpfes: alles, was nicht JSON ist, geht
/// byteweise hinaus. Der Server sagt es selbst in seiner Beschreibung,
/// also wird es gelesen und nicht geraten.
fn wants_file(operation: &serde_json::Value) -> bool {
    let Some(content) = operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(|content| content.as_object())
    else {
        return false;
    };
    !content.is_empty() && !content.contains_key("application/json")
}

/// The declared type of a file body, where the server names one.
///
/// `application/octet-stream` names nothing: the server takes the bytes
/// and looks at them itself.
fn file_type(operation: &serde_json::Value) -> String {
    operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(|content| content.as_object())
        .and_then(|content| content.keys().next())
        .filter(|kind| {
            kind.as_str() != "application/json" && kind.as_str() != "application/octet-stream"
        })
        .cloned()
        .unwrap_or_default()
}

/// The content types of a successful answer.
fn returns(operation: &serde_json::Value) -> Vec<String> {
    let mut kinds = Vec::new();
    if let Some(responses) = operation
        .get("responses")
        .and_then(|value| value.as_object())
    {
        for (status, response) in responses {
            if !status.starts_with('2') {
                continue;
            }
            if let Some(content) = response.get("content").and_then(|value| value.as_object()) {
                for kind in content.keys() {
                    if !kinds.contains(kind) {
                        kinds.push(kind.clone());
                    }
                }
            }
        }
    }
    kinds
}

/// The JSON body's schema, every reference resolved.
fn body_schema(
    document: &serde_json::Value,
    operation: &serde_json::Value,
) -> Option<serde_json::Value> {
    let schema = operation
        .get("requestBody")?
        .get("content")?
        .get("application/json")?
        .get("schema")?;
    Some(resolve(document, schema, &mut Vec::new()))
}

/// Inlines every `$ref` into one self-contained schema.
///
/// **A reference a reader cannot follow is no description.** An agent
/// gets this schema as text and has no `components` to look things up in.
/// A type that contains itself is cut at the second visit and named, so a
/// recursive schema stays finite and still says what it is.
fn resolve(
    document: &serde_json::Value,
    schema: &serde_json::Value,
    seen: &mut Vec<String>,
) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(|value| value.as_str()) {
                let name = reference.rsplit('/').next().unwrap_or("").to_string();
                if seen.contains(&name) {
                    return serde_json::json!({ "type": "object", "description": format!("{name}, wie oben") });
                }
                let Some(target) = document
                    .get("components")
                    .and_then(|c| c.get("schemas"))
                    .and_then(|s| s.get(&name))
                else {
                    return schema.clone();
                };
                seen.push(name);
                let resolved = resolve(document, target, seen);
                seen.pop();
                return resolved;
            }
            serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), resolve(document, value, seen)))
                    .collect(),
            )
        }
        serde_json::Value::Array(list) => serde_json::Value::Array(
            list.iter()
                .map(|value| resolve(document, value, seen))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Welche Felder der Rumpf mindestens braucht.
///
/// Ein Verweis tief in `components` muss dafür aufgelöst werden. Mehr
/// als eine Ebene wird nicht verfolgt: was dann noch fehlt, sagt der
/// Server selbst, und zwar genauer als jede Ableitung hier.
fn required_fields(document: &serde_json::Value, operation: &serde_json::Value) -> Vec<String> {
    let schema = operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
        .and_then(|content| content.get("application/json"))
        .and_then(|json| json.get("schema"));
    let Some(schema) = schema else {
        return Vec::new();
    };
    let resolved = match schema.get("$ref").and_then(|value| value.as_str()) {
        Some(reference) => {
            let name = reference.rsplit('/').next().unwrap_or("");
            document
                .get("components")
                .and_then(|c| c.get("schemas"))
                .and_then(|s| s.get(name))
                .unwrap_or(schema)
        }
        None => schema,
    };
    resolved
        .get("required")
        .and_then(|value| value.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> serde_json::Value {
        json!({
            "paths": {
                "/api/absences": {
                    "get": {"tags": ["http::absences"], "operationId": "list_absences",
                            "summary": "Alle Abwesenheiten"},
                    "post": {"tags": ["http::absences"], "operationId": "create_absence",
                             "requestBody": {"content": {"application/json":
                                 {"schema": {"$ref": "#/components/schemas/NewAbsence"}}}}}
                },
                "/api/absences/calendars": {
                    "get": {"tags": ["http::absences"], "operationId": "list_calendars"}
                },
                "/api/customers/{id}": {
                    "get": {"tags": ["http::customers"], "operationId": "get_customer",
                            "parameters": [
                                {"name": "id", "in": "path", "required": true},
                                {"name": "full", "in": "query", "required": false}
                            ]}
                }
            },
            "components": {"schemas": {"NewAbsence": {"required": ["type", "from"]}}}
        })
    }

    #[test]
    fn the_group_loses_the_module_path() {
        // utoipa schreibt http::customers; auf der Kommandozeile heißt
        // das customers.
        let catalog = parse(&document()).unwrap();
        assert!(
            catalog.groups.contains_key("customers"),
            "{:?}",
            catalog.groups.keys()
        );
        assert!(!catalog.groups.keys().any(|key| key.contains("::")));
    }

    #[test]
    fn the_repetition_in_the_name_falls_away() {
        let catalog = parse(&document()).unwrap();
        assert!(catalog.find("absences", "list").is_some());
        assert!(catalog.find("absences", "create").is_some());
    }

    #[test]
    fn a_name_that_says_something_stays() {
        // list-calendars ist keine Wiederholung der Gruppe, also bleibt
        // es stehen.
        let catalog = parse(&document()).unwrap();
        assert!(catalog.find("absences", "list-calendars").is_some());
    }

    #[test]
    fn path_and_query_are_told_apart() {
        let catalog = parse(&document()).unwrap();
        let operation = catalog.find("customers", "get").unwrap();
        assert_eq!(operation.path_params.len(), 1);
        assert_eq!(operation.path_params[0].name, "id");
        assert_eq!(operation.query_params.len(), 1);
        assert_eq!(operation.query_params[0].name, "full");
    }

    #[test]
    fn the_required_fields_come_through_the_reference() {
        let catalog = parse(&document()).unwrap();
        let operation = catalog.find("absences", "create").unwrap();
        assert_eq!(operation.required_fields, vec!["type", "from"]);
        assert!(operation.has_body);
    }

    #[test]
    fn the_body_schema_is_resolved_all_the_way_down() {
        // A reference inside a reference: an agent sees the inner fields,
        // not a name it cannot look up.
        let document = json!({
            "paths": {"/api/themes/{id}/versions": {"post": {
                "tags": ["http::themes"], "operationId": "add_version",
                "summary": "Speichern legt eine neue Fassung an.",
                "description": "Erwartet den ganzen Satz.",
                "requestBody": {"content": {"application/json":
                    {"schema": {"$ref": "#/components/schemas/NewVersion"}}}}
            }}},
            "components": {"schemas": {
                "NewVersion": {"type": "object", "required": ["spec"],
                    "properties": {"spec": {"$ref": "#/components/schemas/Spec"}}},
                "Spec": {"type": "object", "properties": {
                    "stil": {"type": "string", "enum": ["haarlinien", "zebra"]}}}
            }}
        });
        let catalog = parse(&document).unwrap();
        let operation = catalog.find("themes", "add-version").unwrap();
        let schema = operation.body_schema.as_ref().unwrap();
        assert_eq!(
            schema["properties"]["spec"]["properties"]["stil"]["enum"][1],
            "zebra"
        );
        assert!(!schema.to_string().contains("$ref"), "{schema}");
        assert_eq!(
            operation.description,
            "Speichern legt eine neue Fassung an.\n\nErwartet den ganzen Satz."
        );
    }

    #[test]
    fn a_type_that_contains_itself_stays_finite() {
        let document = json!({
            "paths": {"/api/x": {"post": {"tags": ["http::x"], "operationId": "make",
                "requestBody": {"content": {"application/json":
                    {"schema": {"$ref": "#/components/schemas/Node"}}}}}}},
            "components": {"schemas": {"Node": {"type": "object", "properties": {
                "children": {"type": "array", "items": {"$ref": "#/components/schemas/Node"}}}}}}
        });
        let catalog = parse(&document).unwrap();
        let schema = catalog
            .find("x", "make")
            .unwrap()
            .body_schema
            .clone()
            .unwrap();
        assert!(schema.to_string().contains("Node, wie oben"), "{schema}");
    }

    #[test]
    fn what_comes_back_is_named_by_type() {
        let document = json!({
            "paths": {"/api/documents/{id}/page": {"get": {
                "tags": ["http::documents"], "operationId": "page_image",
                "responses": {"200": {"description": "PNG", "content": {"image/png": {}}},
                              "404": {"description": "nope"}}}}}
        });
        let catalog = parse(&document).unwrap();
        let operation = catalog.find("documents", "page-image").unwrap();
        assert_eq!(operation.returns, vec!["image/png"]);
    }

    #[test]
    fn a_document_without_operations_is_refused_rather_than_silently_empty() {
        // Ein Werkzeug ohne einen einzigen Befehl sieht aus wie ein
        // kaputtes Werkzeug, und das wäre die falsche Diagnose.
        let empty = json!({"paths": {}});
        assert!(parse(&empty).is_err());
    }

    #[test]
    fn two_operations_wanting_the_same_name_both_keep_one() {
        let clash = json!({
            "paths": {
                "/a": {"get": {"tags": ["http::x"], "operationId": "do_x"}},
                "/b": {"post": {"tags": ["http::x"], "operationId": "do_x"}}
            }
        });
        let catalog = parse(&clash).unwrap();
        assert_eq!(catalog.count(), 2, "{:?}", catalog.groups);
        assert!(catalog.find("x", "get-do").is_some());
        assert!(catalog.find("x", "post-do").is_some());
    }
}

#[cfg(test)]
mod file_body_tests {
    use super::*;

    /// Ein Rumpf, der keine JSON-Sorte führt, will eine Datei.
    ///
    /// **Sonst böte die Kommandozeile einen Befehl an, den sie nicht
    /// ausführen kann.** Genau das war bis zum 18.09.2026 der Fall: das
    /// Hochladen eines Papiers stand im Baum, und `--data` hätte die
    /// Bytes durch eine Zeichenkette gezwängt.
    #[test]
    fn an_octet_stream_body_asks_for_a_file() {
        let document = serde_json::json!({
            "paths": {
                "/api/papers/upload": {
                    "post": {"tags": ["http::papers"], "operationId": "upload",
                             "requestBody": {"content": {"application/octet-stream": {}}}}
                },
                "/api/customers": {
                    "post": {"tags": ["http::customers"], "operationId": "create",
                             "requestBody": {"content": {"application/json": {}}}}
                },
                "/api/customers/{id}": {
                    "get": {"tags": ["http::customers"], "operationId": "get",
                            "parameters": [{"name": "id", "in": "path", "required": true}]}
                }
            }
        });
        let catalog = parse(&document).unwrap();
        let von = |gruppe: &str, name: &str| {
            catalog
                .find(gruppe, name)
                .unwrap_or_else(|| panic!("{gruppe} {name} fehlt"))
        };
        assert!(von("papers", "upload").wants_file);
        assert!(!von("customers", "create").wants_file);
        // Ohne Rumpf ist auch keine Datei gemeint.
        assert!(!von("customers", "get").wants_file);
    }
}
