//! xenonet completion <zsh|bash> (cli/commands/completion.js): tab
//! completion for the xenonet command, from its command table.

use super::theme::BUILT_IN;
use super::COMMANDS;
use crate::args::Flags;
use crate::out::{print, Done, Fail};

pub const HELP: &str = include_str!("help/completion.txt");

const FLAGS: [&str; 7] = ["--json", "--dev", "--demo", "--port", "--no-color", "--help", "--version"];

// Each command's own words. The apps connect and hook take are the
// registry's.
fn args() -> Vec<(&'static str, String)> {
    let themes = [BUILT_IN.as_slice(), &["copy"]].concat().join(" ");
    let apps = xenonet_base::apps::with("connect").map(|a| a.id.as_str()).collect::<Vec<_>>().join(" ");
    vec![
        ("connect", apps.clone()),
        ("hook", apps),
        ("completion", "zsh bash".into()),
        ("doctor", "--only".into()),
        ("drift", "--all --exit-code".into()),
        ("console", "--instance".into()),
        ("theme", themes),
        ("actor", "list show register heartbeat status event targets inbox send leave remove observe".into()),
        ("hosts", "self add remove test reconnect".into()),
        ("net", "create join status hosts add-machine remove replace leave set approve gateway".into()),
        ("svc", "publish unpublish list".into()),
    ]
}

pub fn run(words: &[String], _flags: &Flags) -> Done {
    let shown: Vec<_> = COMMANDS.iter().filter(|c| !c.hidden).collect();
    match words.first().map(String::as_str) {
        Some("zsh") => {
            let list: Vec<String> = shown.iter().map(|c| format!("    '{}:{}'", c.name, c.about().replace('\'', "'\\''"))).collect();
            let cases: Vec<String> = args().iter().map(|(k, v)| format!("      {k}) compadd -- {v} ;;")).collect();
            print(format!(
                "#compdef xenonet
_xenonet() {{
  local -a commands
  commands=(
{}
  )
  if (( CURRENT == 2 )); then
    _describe 'command' commands
    compadd -- {}
  else
    case $words[2] in
{}
      *) _files ;;
    esac
  fi
}}
compdef _xenonet xenonet",
                list.join("\n"),
                FLAGS.join(" "),
                cases.join("\n")
            ));
            Ok(0)
        }
        Some("bash") => {
            let cases: Vec<String> = args().iter().map(|(k, v)| format!("    {k}) COMPREPLY=($(compgen -W \"{v}\" -- \"$cur\")) ;;")).collect();
            let names: Vec<&str> = shown.iter().map(|c| c.name).collect();
            print(format!(
                "_xenonet() {{
  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"
  if [ \"$COMP_CWORD\" -eq 1 ]; then
    COMPREPLY=($(compgen -W \"{} {}\" -- \"$cur\"))
    return
  fi
  case \"${{COMP_WORDS[1]}}\" in
{}
    *) COMPREPLY=($(compgen -f -- \"$cur\")) ;;
  esac
}}
complete -o default -F _xenonet xenonet",
                names.join(" "),
                FLAGS.join(" "),
                cases.join("\n")
            ));
            Ok(0)
        }
        _ => Err(Fail::new("say which shell: xenonet completion zsh, or xenonet completion bash", 2)),
    }
}
