export function openRouterView(value) {
  if (!value) return { setup: false, value: "Unavailable", icon: "none", hint: "OpenRouter usage has not been read. XenoNet will retry while this window is open." };
  if (value.key === "missing") return { setup: true, value: "Set key", icon: "key", hint: "No OpenRouter key is stored in XenoNet. Set the OpenRouter key for decision models. Usage covers that key, not other apps or the whole account." };
  if (value.error || value.key === "rejected") return { setup: value.key === "rejected", value: value.key === "rejected" ? "Check key" : "Unavailable", icon: value.key === "rejected" ? "key-bad" : "none", hint: value.error || "OpenRouter rejected this key." };
  return null;
}
