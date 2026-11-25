// exit: 1
// A ref argument of a task that can wait: passed by copy here, which differs from a reference
// while the task waits, so it is rejected.
module t;
  task automatic waits(ref int x); #1 x = 3; endtask
  initial begin int z; waits(z); end
endmodule
