// exit: 1
// An event is not a value that can be passed to a task or function.
module test;
  event e;
  task t(input x);
  endtask
  initial t(e);
endmodule
