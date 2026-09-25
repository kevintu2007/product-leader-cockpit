if (process.env.PMC_VERIFY_FAILURE_PROBE === "1") {
  console.error("[failure-probe] controlled constituent failure");
  process.exit(97);
}

console.log("[failure-probe] pass");
