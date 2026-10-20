// Drawn in software (XENONET_SOFTWARE_RENDERING=1, as on a Linux CI runner
// with no GPU), XenoNet Dev's map makes a frame every second or two, and
// each step an end-to-end test takes waits for the view to settle on one.
// A test's time limit guards against a hang rather than measuring speed, so
// there it stretches to match.
export const software = process.env.XENONET_SOFTWARE_RENDERING === "1";
export const limit = (ms) => (software ? ms * 3 : ms);
