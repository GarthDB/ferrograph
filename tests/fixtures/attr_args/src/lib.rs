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

#[command(version = my_version())]
struct Versioned;

fn parse_hex() {}
fn parse_pair() {}
fn default_name() {}
fn mapper() {}
fn is_default() {}
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
