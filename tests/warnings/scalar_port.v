// flags: -gno-io-range-error
// A port declared without a range that a net or variable declaration gives one: with
// -gno-io-range-error a warning (by module, then for functions and tasks by name).
module m(a, b, c[1:0]);
  input a;
  output b;
  input c;
  wire [7:0] a;
  reg [3:0] b;
  wire [2:0] c;
  function [4:0] zf;
    input l, r;
    reg [4:0] l;
    integer r;
    zf = l + r;
  endfunction
  function [7:0] af;
    input l;
    reg [7:0] l;
    af = l;
  endfunction
  initial b = zf(a[4:0], 1) + af(8'd2);
endmodule

module test;
  wire [7:0] a = 8'd5;
  wire [3:0] b;
  m u(a, b, 2'b10);
  initial #1 $display("b=%0d c=%b", b, u.c);
endmodule
