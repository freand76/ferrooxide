// exit: 1
// A ref argument of a task that can wait, given a variable of the calling subroutine that
// starts a join_none branch: the branch can use the variable while the task waits, which
// differs from passing it by copy, so it is rejected.
module t;
  task automatic waits(ref int x); #1 x = 3; endtask
  task automatic caller();
    int z;
    fork #1 z = 5; join_none
    waits(z);
  endtask
  initial caller();
endmodule
