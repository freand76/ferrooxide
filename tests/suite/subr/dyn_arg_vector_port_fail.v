// exit: 1
// a dynamic array is not assignment compatible with a vector (IEEE 1800-2017 7.5, 13.3):
// it cannot be the actual of a vector output argument
module test;
  logic [7:0] bf [];
  task t1(output logic [7:0] buffer);
    buffer = 0;
  endtask
  initial t1(bf);
endmodule
