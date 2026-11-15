// Packed multi-dimensional arrays as subroutine ports and block variables: the whole vector
// is passed, and element selects index the outer dimension (IEEE 1800 7.4.1).
module test;
  reg failed = 0;

  task t(input logic [3:0][7:0] x, output logic [1:0][3:0] y);
    if (x !== 32'h04030201 || x[1] !== 8'h02 || $bits(x) != 32) failed = 1;
    y = {x[3][3:0], x[2][3:0]};
  endtask

  function logic [15:0] f(input logic [1:0][7:0] a);
    logic [1:0][7:0] b;
    b[0] = a[1];
    b[1] = a[0];
    return b;
  endfunction

  initial begin
    logic [3:0][7:0] v;
    logic [1:0][3:0] r;
    v[0] = 8'h1;
    v[1] = 8'h2;
    v[2] = 8'h3;
    v[3] = 8'h4;
    t(v, r);
    if (r !== 8'h43 || r[1] !== 4'h4) failed = 1;
    if (f(16'habcd) !== 16'hcdab) failed = 1;
    if (failed) $display("FAILED");
    else $display("PASSED");
  end
endmodule
