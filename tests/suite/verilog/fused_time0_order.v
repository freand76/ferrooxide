// expected
// A chain of continuous assignments that fusion merges into one process (port connection to
// a part select, then a part-select assignment) settles before the processes that read its
// result start waiting: the `always` sees no change at time 0, as in the reference.
module d(B);
  input [1:0] B;
  wire [2:1] A;
  assign A[2:1] = B;
  always @(A) $display("%0t A=%b", $time, A);
endmodule

module test;
  reg [2:0] A;
  d dd(A[1:0]);
  initial begin
    #1 A = 1;
    #1 A = 3'b110;
  end
endmodule
