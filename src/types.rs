//! Question type definitions, ported from pyxform's `question_type_dictionary.py`.

/// Static definition of how a question type maps to XForm control + bind.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeDef {
    /// Body control tag (`input`, `select1`, `select`, `upload`, `trigger`,
    /// `range`, `odk:rank`). `None` means the type has no body element.
    pub control_tag: Option<&'static str>,
    /// `mediatype` attribute for `upload` controls.
    pub mediatype: Option<&'static str>,
    /// Value for the bind `type` attribute.
    pub bind_type: Option<&'static str>,
    /// Whether the bind carries `readonly="true()"`.
    pub readonly: bool,
    /// `(jr:preload, jr:preloadParams)` for preloaded/metadata types.
    pub preload: Option<(&'static str, &'static str)>,
    /// Built-in constraint expression.
    pub constraint: Option<&'static str>,
    /// Built-in hint text.
    pub hint: Option<&'static str>,
}

const fn input(bind_type: &'static str) -> TypeDef {
    TypeDef {
        control_tag: Some("input"),
        bind_type: Some(bind_type),
        ..EMPTY
    }
}

const fn upload(mediatype: &'static str) -> TypeDef {
    TypeDef {
        control_tag: Some("upload"),
        mediatype: Some(mediatype),
        bind_type: Some("binary"),
        ..EMPTY
    }
}

const fn preload(kind: &'static str, params: &'static str, bind_type: &'static str) -> TypeDef {
    TypeDef {
        bind_type: Some(bind_type),
        preload: Some((kind, params)),
        ..EMPTY
    }
}

const EMPTY: TypeDef = TypeDef {
    control_tag: None,
    mediatype: None,
    bind_type: None,
    readonly: false,
    preload: None,
    constraint: None,
    hint: None,
};

/// Names offered in "did you mean ...?" suggestions when a type cell does
/// not parse. Covers the plain types plus the structural/select prefixes the
/// parser handles before consulting this module.
pub const SUGGESTIBLE_TYPES: [&str; 44] = [
    "text",
    "integer",
    "decimal",
    "range",
    "date",
    "time",
    "dateTime",
    "note",
    "trigger",
    "acknowledge",
    "geopoint",
    "geotrace",
    "geoshape",
    "barcode",
    "photo",
    "image",
    "audio",
    "video",
    "file",
    "calculate",
    "hidden",
    "audit",
    "start",
    "end",
    "today",
    "deviceid",
    "phonenumber",
    "username",
    "email",
    "start-geopoint",
    "background-audio",
    "background-geopoint",
    "xml-external",
    "csv-external",
    "select_one",
    "select_multiple",
    "select_one_from_file",
    "select_multiple_from_file",
    "rank",
    "begin group",
    "end group",
    "begin repeat",
    "end repeat",
    "osm",
];

/// Look up a (canonicalized) question type. Select types are handled by the
/// parser before reaching here, because they carry a list name argument.
pub fn lookup(qtype: &str) -> Option<TypeDef> {
    let def = match qtype {
        "text" | "string" => input("string"),
        "integer" | "int" => input("int"),
        "decimal" => input("decimal"),
        "date" => input("date"),
        "time" => input("time"),
        "datetime" | "dateTime" | "date time" => input("dateTime"),
        "geopoint" | "gps" | "location" => input("geopoint"),
        "geoshape" => input("geoshape"),
        "geotrace" => input("geotrace"),
        "barcode" => input("barcode"),

        "note" => TypeDef {
            readonly: true,
            ..input("string")
        },
        "trigger" => TypeDef {
            control_tag: Some("trigger"),
            ..EMPTY
        },
        "acknowledge" => TypeDef {
            control_tag: Some("trigger"),
            bind_type: Some("string"),
            ..EMPTY
        },

        "select one" => TypeDef {
            control_tag: Some("select1"),
            bind_type: Some("string"),
            ..EMPTY
        },
        "select all that apply" => TypeDef {
            control_tag: Some("select"),
            bind_type: Some("string"),
            ..EMPTY
        },
        "select one external" => input("string"),
        "rank" => TypeDef {
            control_tag: Some("odk:rank"),
            bind_type: Some("odk:rank"),
            ..EMPTY
        },

        "photo" | "image" => upload("image/*"),
        "audio" => upload("audio/*"),
        "video" => upload("video/*"),
        "file" => upload("application/*"),
        "osm" => upload("osm/*"),

        "calculate" => TypeDef {
            bind_type: Some("string"),
            ..EMPTY
        },
        "hidden" => TypeDef {
            bind_type: Some("string"),
            ..EMPTY
        },
        "audit" => TypeDef {
            bind_type: Some("binary"),
            ..EMPTY
        },

        "range" => TypeDef {
            control_tag: Some("range"),
            bind_type: Some("int"),
            ..EMPTY
        },

        "start" | "start time" => preload("timestamp", "start", "dateTime"),
        "end" | "end time" => preload("timestamp", "end", "dateTime"),
        "today" | "get today" => preload("date", "today", "date"),
        "deviceid" | "device id" | "imei" | "get device id" => {
            preload("property", "deviceid", "string")
        }
        "subscriberid" | "subscriber id" | "get subscriber id" => {
            preload("property", "subscriberid", "string")
        }
        "simserial" | "sim id" | "get sim id" => preload("property", "simserial", "string"),
        "phonenumber" | "get phone number" => preload("property", "phonenumber", "string"),
        "username" => preload("property", "username", "string"),
        "email" => preload("property", "email", "string"),
        "uri:deviceid" => preload("property", "uri:deviceid", "string"),
        "uri:subscriberid" => preload("property", "uri:subscriberid", "string"),
        "uri:simserial" => preload("property", "uri:simserial", "string"),
        "uri:phonenumber" => preload("property", "uri:phonenumber", "string"),
        "uri:username" => preload("property", "uri:username", "string"),
        "uri:email" => preload("property", "uri:email", "string"),

        "start-geopoint" => TypeDef {
            control_tag: Some("action"),
            bind_type: Some("geopoint"),
            ..EMPTY
        },
        "background-geopoint" => TypeDef {
            control_tag: Some("action"),
            bind_type: Some("geopoint"),
            ..EMPTY
        },
        "background-audio" => TypeDef {
            control_tag: Some("action"),
            bind_type: Some("binary"),
            ..EMPTY
        },

        "phone number" => TypeDef {
            constraint: Some("regex(., '^\\d*$')"),
            hint: Some("Enter numbers only."),
            ..input("string")
        },
        "percentage" => TypeDef {
            constraint: Some("0 <= . and . <= 100"),
            ..input("int")
        },

        "xml-external" | "csv-external" => EMPTY,

        _ => return None,
    };
    Some(def)
}
