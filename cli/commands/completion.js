// xenonet completion <zsh|bash>: tab completion for the xenonet command, from
// its command table (../lib/commands.js).
import { print, Fail } from "../lib/out.js";
import { COMMANDS } from "../lib/commands.js";
import { BUILT_IN } from "../../core/themes.js";

export const help = `Prints tab completion for your shell. Add a line to your shell's profile:

  zsh   eval "$(xenonet completion zsh)"     (in ~/.zshrc, after compinit)
  bash  eval "$(xenonet completion bash)"    (in ~/.bashrc)`;

const FLAGS = ["--json", "--dev", "--demo", "--port", "--no-color", "--help", "--version"];
const ARGS = { connect: "claude codex", hook: "claude codex", completion: "zsh bash", doctor: "--only", drift: "--all --exit-code", console: "--instance", theme: [...BUILT_IN, "copy"].join(" "), actor: "list show register heartbeat status event targets inbox send leave remove observe", hosts: "self add remove test reconnect" };

export async function run([shell]) {
  const shown = Object.entries(COMMANDS).filter(([, v]) => !v.hidden);
  if (shell === "zsh") {
    const list = shown.map(([k, v]) => `    '${k}:${v.about.replace(/'/g, "'\\''")}'`).join("\n");
    const cases = Object.entries(ARGS).map(([k, v]) => `      ${k}) compadd -- ${v} ;;`).join("\n");
    return print(`#compdef xenonet
_xenonet() {
  local -a commands
  commands=(
${list}
  )
  if (( CURRENT == 2 )); then
    _describe 'command' commands
    compadd -- ${FLAGS.join(" ")}
  else
    case $words[2] in
${cases}
      *) _files ;;
    esac
  fi
}
compdef _xenonet xenonet`);
  }
  if (shell === "bash") {
    const cases = Object.entries(ARGS).map(([k, v]) => `    ${k}) COMPREPLY=($(compgen -W "${v}" -- "$cur")) ;;`).join("\n");
    return print(`_xenonet() {
  local cur="\${COMP_WORDS[COMP_CWORD]}"
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "${shown.map(([k]) => k).join(" ")} ${FLAGS.join(" ")}" -- "$cur"))
    return
  fi
  case "\${COMP_WORDS[1]}" in
${cases}
    *) COMPREPLY=($(compgen -f -- "$cur")) ;;
  esac
}
complete -o default -F _xenonet xenonet`);
  }
  throw new Fail("say which shell: xenonet completion zsh, or xenonet completion bash", 2);
}
