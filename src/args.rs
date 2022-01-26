use clap::{App, Arg};

#[derive(Debug)]
pub struct Args {
    pub source: String,
    pub target: String,
    pub topics: Vec<String>,
    pub quiet: bool,
}

impl Args {
    pub fn parse() -> Self {
        let matches = App::new("nuts")
            .version("0.1.0-alpha")
            .author("Marquitos <https://github.com/sonirico>")
            .about("NATS.io proxy")
            .arg(
                Arg::new("source")
                    .short('s')
                    .long("source")
                    .takes_value(true)
                    .required(true)
                    .help("Source nats to read from"),
            )
            .arg(
                Arg::new("target")
                    .short('d')
                    .long("destination")
                    .takes_value(true)
                    .required(true)
                    .help("Destination nats to write to"),
            )
            .arg(
                Arg::new("topics")
                    .short('t')
                    .long("topics")
                    .min_values(1)
                    .takes_value(true)
                    .multiple_values(true)
                    .help("Topics to relay"),
            )
            .arg(
                Arg::new("quiet")
                    .short('q')
                    .long("quiet")
                    .takes_value(false)
                    .help("Disable progress output"),
            )
            .get_matches();

        let source = matches.value_of("source").unwrap_or_default().to_string();
        let target = matches.value_of("target").unwrap_or_default().to_string();
        let topics: Vec<String> = matches
            .values_of("topics")
            .unwrap_or_default()
            .collect::<Vec<&str>>()
            .iter()
            .map(|&x| String::from(x))
            .collect::<Vec<String>>();
        let quiet = matches.is_present("quiet");

        Self {
            source,
            target,
            topics,
            quiet,
        }
    }
}
