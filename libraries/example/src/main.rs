#![feature(extern_prelude)]
extern crate rand;
extern crate clap;
extern crate tokio;
#[macro_use]
extern crate futures;
extern crate gossip;

mod simulate;

use clap::{SubCommand, App, Arg};

fn main() {
    let matches = App::new("Peer simulator")
        .version("0.1")
        .about("A simulator for peer-to-peer networks")
        .subcommand(SubCommand::with_name("simulate")
            .about("simulates a P2P network")
            .arg(Arg::with_name("nodes")
                 .short("n")
                 .long("nodes")
                 .required(true)
                 .takes_value(true)
            )
        )
        .subcommand(SubCommand::with_name("display")
            .about("display the results of a P2P network")
            .arg(Arg::with_name("file")
                 .short("f")
                 .long("files")
            )
        ).get_matches();

    if let Some(matches) = matches.subcommand_matches("simulate") {
        let num_nodes = matches.value_of("nodes")
            .and_then(|x| x.parse::<usize>().ok())
            .unwrap_or(100);

        simulate::start(num_nodes);
    }
}
