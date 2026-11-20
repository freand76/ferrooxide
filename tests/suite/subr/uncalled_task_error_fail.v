// exit: 1
// A task that is never called is still checked: a task returns no value.
module test;
  task t;
    return 1;
  endtask
endmodule
