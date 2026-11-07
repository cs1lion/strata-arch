const React = require("react");
const { readFileSync } = require("node:fs");

function cjsMember(value) {
  return React.useMemo(() => value, [value]);
}

function cjsBare(path) {
  return readFileSync(path);
}

module.exports = { cjsMember, cjsBare };
