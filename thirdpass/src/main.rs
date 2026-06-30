use structopt::StructOpt;

mod command;
mod common;
mod extension;
mod package;
mod peer;
mod registry;
mod review;
#[cfg(test)]
mod test_support;

fn main() {
    let env = env_logger::Env::new().filter_or("THIRDPASS_LOG", "off");
    env_logger::Builder::from_env(env)
        .filter_module("tokei::language::language_type", log::LevelFilter::Error)
        .filter_module("h2", log::LevelFilter::Info)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let (thirdpass_args, extension_args) = split_extension_args(&args);
    let commands = command::Opts::from_iter(thirdpass_args.iter());

    match command::run_command(commands.command, &extension_args) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("{}", format_error_chain(&e));
            std::process::exit(-2)
        }
    }
}

fn format_error_chain(error: &anyhow::Error) -> String {
    let mut chain = error.chain();
    let Some(root) = chain.next() else {
        return error.to_string();
    };

    let mut message = root.to_string();
    for cause in chain {
        message.push_str("\nCaused by: ");
        message.push_str(&cause.to_string());
    }
    message
}

/// Arguments after -- are passed to extensions.
fn split_extension_args(args: &Vec<String>) -> (Vec<String>, Vec<String>) {
    let split_element = "--";
    let mut pre_split = vec![];
    let mut post_split = vec![];

    let mut split_point_found = false;
    for arg in args {
        if arg == split_element {
            split_point_found = true;
            continue;
        }
        if !split_point_found {
            pre_split.push(arg.clone());
        } else {
            post_split.push(arg.clone());
        }
    }
    (pre_split, post_split)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn format_error_chain_includes_causes() {
        let error = std::fs::read_to_string("/definitely/not/a/thirdpass/file")
            .context("outer context")
            .expect_err("missing file should fail");

        let formatted = format_error_chain(&error);

        assert!(formatted.starts_with("outer context"));
        assert!(formatted.contains("Caused by:"));
        assert!(formatted.contains("No such file or directory"));
    }
}
