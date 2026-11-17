// Ironswift's testharnessreport.js: replaces the WPT one in a local checkout so results
// come out on the console, where tools/wpt.py reads them.
setup({ output: false, explicit_timeout: true });
add_completion_callback(function (tests, status) {
  var out = { status: status.status, message: status.message || "", tests: [] };
  for (var i = 0; i < tests.length; i++) {
    out.tests.push({ name: tests[i].name, status: tests[i].status, message: tests[i].message || "" });
  }
  console.log("WPT-RESULT " + JSON.stringify(out));
  // The browser's waiting mode (tools/wpt.py) ends the run once this is set.
  globalThis.__wptDone = true;
});
