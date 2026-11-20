// exit: 1
// A typed declaration of a port goes only with a port declaration without a range.
module test;
  task t;
    output [31:0] x;
    integer x;
    x = 1;
  endtask
endmodule
