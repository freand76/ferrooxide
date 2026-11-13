// iverilog: -g2005
// SystemVerilog keywords used as Verilog-2005 identifiers
module add1 (input a, output y);
  assign y = ~a;
endmodule

module top;
  reg [2:0] bit;
  reg int;
  wire byte;
  reg do, priority;
  add1 final (int, byte);
  initial begin
    bit = 3'd5;
    int = 1'b0;
    do <= 1;
    priority = bit[1];
    #1 $display("%d %b %b %b %b", bit, int, byte, do, priority);
    bit = bit + int;
    begin : return
      $display("%m %d", bit);
    end
  end
endmodule
