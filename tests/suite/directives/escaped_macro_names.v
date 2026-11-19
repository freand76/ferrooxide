// Macros named by escaped identifiers (IEEE 1800-2017 22.5.1: a text macro name is an
// identifier, escaped ones included): `\name is `name, names with backticks, and an
// escaped name with parameters after the white space that ends it.
`define plain "plain"
`define \esc "esc"
`define \`tick "tick"
`define \wrap (x) {x, x}
module m;
  initial begin
    if (`\plain != "plain" || `esc != "esc" || `\esc != "esc" || `\`tick != "tick")
      $display("FAILED -- names");
    if (`\wrap (4'h5) != 8'h55) $display("FAILED -- parameters");
    $display("PASSED");
  end
endmodule
