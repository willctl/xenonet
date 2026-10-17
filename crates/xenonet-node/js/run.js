// A `run` (a command's output, or it throws), as the Rust calls it: the
// output, or the error's message, with the error kept. When a command's
// failure ends the call, that error is thrown again, as JS would have let
// it through; the Rust's own errors are thrown with its words.
export function withRun(call, run) {
  let thrown;
  const answer = call((argv) => {
    try {
      return { out: String(run(argv[0], argv.slice(1))) };
    } catch (err) {
      thrown = err;
      return { error: String(err?.message ?? err) };
    }
  });
  if (!answer.error) return answer.ok;
  if (answer.error.kind === "run") throw thrown;
  throw Object.assign(new Error(answer.error.message), answer.error.code ? { code: answer.error.code } : {});
}
