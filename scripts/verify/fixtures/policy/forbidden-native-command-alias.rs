use std::process::Command as OtherCommand;

fn spawn_unreviewed_process() {
    let _child = OtherCommand::new("unreviewed-helper");
}
