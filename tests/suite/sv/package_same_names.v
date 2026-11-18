// `pkg::name` names the parameter of that package even when another package declares the
// same name (IEEE 1800-2017 26.3).
package p1;
  localparam int step = 1;
  localparam [5:2] R = 4'b1001;
  typedef enum { A, B } e_t;
endpackage

package p2;
  localparam int step = 2;
  localparam [5:2] R = 4'b0110;
  typedef enum { C = 7, D } e_t;
endpackage

module m;
  logic [p2::step*4-1:0] v;
  p2::e_t e = p2::D;
  initial begin
    if (p1::step != 1 || p2::step != 2) $display("FAILED -- step %0d %0d", p1::step, p2::step);
    if (p1::R[5] !== 1'b1 || p2::R[5:4] !== 2'b01) $display("FAILED -- range selects");
    if ($bits(v) != 8) $display("FAILED -- $bits(v)=%0d", $bits(v));
    if (e != 8 || p1::B != 1) $display("FAILED -- enum %0d %0d", e, p1::B);
    $display("PASSED");
  end
endmodule
