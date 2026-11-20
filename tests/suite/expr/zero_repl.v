// A replication with a zero count adds nothing to a concatenation (IEEE 1800-2017
// 11.4.12.1).
module test;
  parameter N = 0;
  reg [11:0] r;
  wire [3:0] a = 4'h5;
  initial begin
    #1 r = {{N{1'b1}}, a, {2{a, {N{1'b0}}}}};
    if (r === 12'h555 && $bits({a, {N{a}}}) == 4) $display("PASSED");
    else $display("FAILED %h", r);
  end
endmodule
