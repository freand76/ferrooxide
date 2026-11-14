// Type parameters (overrides are evaluated where the instance is), and constants in types of
// packages and the file scope evaluated where the type is declared.
localparam W = 3;
typedef logic [W-1:0] unit_t;
package P;
  localparam X = 8;
  typedef enum logic [X-1:0] { A, B = X } T;
endpackage

module M #(parameter type T1 = integer, parameter WIDTH = 32, type T2 = real) (input T1 d, output T1 q);
  localparam A = 2;
  localparam type T3 = logic [A:0];
  T1 x;
  T2 r = 1.5;
  T3 z;
  assign q = ~d;
  initial #1 $display("%m %0d %0d %0d %0d %f", $bits(x), $bits(T1), WIDTH, $bits(z), r);
endmodule

module top;
  localparam A = 4;
  localparam W = 9;
  typedef logic [A-1:0] T2;
  typedef int T;
  wire [31:0] q1; wire [15:0] q2; wire [3:0] q3, q4; wire [1:0] q5;
  M i1(.d(32'd5), .q(q1));
  M #(.T1(logic [15:0]), .WIDTH(16)) i2(.d(16'h00ff), .q(q2));
  M #(.T1(logic [A-1:0]), .WIDTH(4)) i3(.d(4'h3), .q(q3));
  M #(T2, 4) i4(.d(4'h9), .q(q4));
  M #(.T1(bit [1:0]), .WIDTH(2), .T2(int)) i5(.d(2'b01), .q(q5));
  P::T e = P::B;
  unit_t u;
  initial #2 $display("%h %h %h %h %h %0d %0d %0d", q1, q2, q3, q4, q5, e, $bits(e), $bits(u));
endmodule
