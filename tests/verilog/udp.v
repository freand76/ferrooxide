// User-defined primitives: combinational (with x and b), edge-triggered flip-flop with initial
// value, level-sensitive latch, and an instance delay.
primitive mux2 (out, sel, a, b);
  output out;
  input sel, a, b;
  table
    // sel a b : out
       0   0 ? : 0;
       0   1 ? : 1;
       1   ? 0 : 0;
       1   ? 1 : 1;
       x   0 0 : 0;
       x   1 1 : 1;
  endtable
endprimitive

primitive dff (q, clk, d);
  output q;
  reg q;
  input clk, d;
  initial q = 1'b0;
  table
    // clk d : q : q+
       r   0 : ? : 0;
       r   1 : ? : 1;
       (?0) ? : ? : -;
       ?   * : ? : -;
  endtable
endprimitive

primitive latch (output reg q, input en, input d);
  table
    1 0 : ? : 0;
    1 1 : ? : 1;
    0 ? : ? : -;
  endtable
endprimitive

module tb;
  reg sel, a, b, clk, d, en;
  wire m, q, l, md;
  mux2 u1 (m, sel, a, b);
  dff  u2 (q, clk, d);
  latch u3 (l, en, d);
  mux2 #3 u4 (md, sel, a, b);
  initial begin
    $monitor("%0t sel=%b a=%b b=%b m=%b md=%b | clk=%b d=%b q=%b | en=%b l=%b", $time, sel, a, b, m, md, clk, d, q, en, l);
    sel = 0; a = 0; b = 1; clk = 0; d = 0; en = 0;
    #5 a = 1;
    #5 sel = 1;
    #5 sel = 1'bx;
    #5 b = 0; a = 0;
    #5 d = 1;
    #5 clk = 1;
    #5 clk = 0; d = 0;
    #5 en = 1;
    #5 d = 1;
    #5 en = 0; d = 0;
    #5 clk = 1'bx;
    #5 clk = 1;
    #5 $finish;
  end
endmodule
