// An answer from the Rust, { ok } or { error }, as the JS would have
// returned or thrown it: the value, or an Error with the error's message
// and its other fields (code, status…).
export function settle(answer) {
  if (!answer.error) return answer.ok;
  throw errorOf(answer.error);
}

export function errorOf(error) {
  if (typeof error === "string") return new Error(error);
  const { message, ...rest } = error;
  return Object.assign(new Error(message), rest);
}
