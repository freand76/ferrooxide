// Unpacked arrays with equivalent element types assign to each other (IEEE 1800-2017
// 6.22.2): integer and logic signed [31:0], int and bit signed [31:0].
module test;
  integer a[2];
  logic signed [31:0] b[2];
  int c[2];
  bit signed [31:0] d[2];
  wire [3:0] w[2];
  logic [3:0] v[2];
  assign w = v;
  initial begin
    b[0] = -3; b[1] = 4;
    d[0] = -5; d[1] = 6;
    v[0] = 1; v[1] = 2;
    a = b;
    c = d;
    #1 if (a[0] == -3 && c[1] == 6 && w[1] == 2) $display("PASSED");
    else $display("FAILED");
  end
endmodule
