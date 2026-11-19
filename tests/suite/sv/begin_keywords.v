// `begin_keywords "1364-2005" (IEEE 1800-2017 22.14): in its region the keywords that
// SystemVerilog added are names, as in Verilog-2005; `end_keywords returns to the
// SystemVerilog keywords. A "1364-1995" region also frees the configuration keywords.
`begin_keywords "1364-2005"
module old(input bit, output logic);
  assign logic = ~bit;
endmodule
`end_keywords

`begin_keywords "1364-1995"
module older;
  reg [3:0] config, cell;
endmodule
`end_keywords

module test;
  logic a, y;
  old u(.bit(a), .logic(y));
  older o();
  initial begin
    a = 0;
    o.config = 5;
    #1;
    if (y === 1'b1 && o.config == 5) $display("PASSED");
    else $display("FAILED");
  end
endmodule
