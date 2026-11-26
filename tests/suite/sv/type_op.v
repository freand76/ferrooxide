// the type operator (IEEE 1800-2017 6.23): the type of an expression or a data type, in
// declarations and in comparisons of types (matching types, 6.22.1)
module sub #(parameter type T = logic [11:0]) (output int r);
  typedef enum {A, B} e_t;
  typedef e_t e2_t;
  typedef enum {C, D} f_t;
  initial begin
    r = 0;
    case (type(T))
      type(logic [11:0]): r = 1;
      type(int): r = 2;
      default: r = 3;
    endcase
    if (type(e_t) != type(e2_t)) r = 10;
    if (type(e_t) == type(f_t)) r = 11;
    if (type(int) == type(bit signed [31:0])) r = 12;
    if (type(T) === type(logic [12:0])) r = 13;
  end
endmodule

module top;
  real a = 4.75;
  real b = 0.75;
  var type(a + b) c;
  logic [3:0] x;
  bit signed [6:0] y;
  int i;
  type(x + y) s;
  type(i) j;
  type(x == y) e;
  type(logic [11:0]) l;
  int r1, r2, r3;
  sub s1(r1);
  sub #(int) s2(r2);
  sub #(byte) s3(r3);
  generate
    if (type(type(i)) == type(int)) begin : g
      initial j = 0;
    end
  endgenerate
  initial begin
    #1;
    c = a + b;
    s = '1;
    j = -1;
    if (c == 5.5 && $bits(s) == 7 && !$isunknown(s) && s == 7'h7f && $bits(j) == 32 && j < 0
        && $bits(e) == 1 && $bits(l) == 12 && r1 == 1 && r2 == 2 && r3 == 3
        && type(c) == type(real) && type(s) == type(logic [6:0]) && type(s) != type(bit [6:0]) && type(j) == type(int))
      $display("PASSED");
    else
      $display("FAILED %f %0d %0d %0d %0d %0d %0d %0d", c, $bits(s), $bits(j), $bits(e), $bits(l), r1, r2, r3);
  end
endmodule
