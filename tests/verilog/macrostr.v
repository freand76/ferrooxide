// Macro strings: `" ... `", `\`" and token pasting
`define MY_FILE(dir, file) `"dir/file`"
`define QUOTE(x) `"x is `\`"x`\`"`"
`define NAME `"plain`"
`define CAT(a, b) a``b
module top;
  reg [7:0] ab;
  initial begin
    `CAT(a, b) = 8'd7;
    $display(`MY_FILE(src, top.v));
    $display(`QUOTE(hello));
    $display(`NAME);
    $display("%0d", ab);
  end
endmodule
