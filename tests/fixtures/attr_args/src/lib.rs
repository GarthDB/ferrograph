#[derive(Parser)]
struct Cli {
    #[arg(value_parser = parse_hex)]
    a: u64,
    #[arg(value_parser = parse_pair::<String, i32>)]
    b: (String, i32),
    #[arg(default_value = default_name())]
    c: String,
    #[arg(value_parser = Wrapper::new().map(mapper))]
    d: bool,
    #[arg(short, long)]
    flag: bool,
}

#[derive(Serialize)]
struct Conf {
    #[serde(skip_serializing_if = "is_default")]
    x: u8,
}

#[derive(Deserialize)]
struct Defaulted {
    #[serde(default = "make_default")]
    y: u8,
    #[serde(rename = "z", default = "make_other", deserialize_with = "de_with")]
    z: u8,
}

#[command(version = my_version())]
struct Versioned;

fn parse_hex() {}
fn parse_pair() {}
fn default_name() {}
fn mapper() {}
fn is_default() {}
fn make_default() {}
fn make_other() {}
fn de_with() {}
fn my_version() {}

// `short` and `long` appear only as bare flags in the attribute above: not references.
fn short() {}
fn long() {}
fn unused_private() {}

#[test]
fn nested_fn_named_in_attribute() {
    fn about() -> &'static str {
        "about"
    }

    #[derive(Parser)]
    enum Sub {
        #[command(about = about())]
        Selected,
    }
}

pub enum Tok {
    #[regex(r"a", parse_a, priority = 2, ignore(case))]
    A,
    #[token("d", callback = parse_d)]
    D,
}

fn parse_a() {}
fn parse_d() {}
// Attribute keywords after a literal are not references.
fn priority() {}
fn ignore() {}
