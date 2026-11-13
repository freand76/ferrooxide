// String literals: NUL bytes and "" with %s / %0s; backslash-newline continuation
module top;
  reg [47:0] s;
  initial begin
    $sformat(s, ":%s:%0s:", "", "");
    $display("[%s]", s);
    $display(":%s:%0s:%x:", "\000a\000b", "\000a\000b", "\000a\000b");
    if ("this matches" == "this\
 matches") $display("continued");
  end
endmodule
