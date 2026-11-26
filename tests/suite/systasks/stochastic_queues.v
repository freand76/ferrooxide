// The stochastic queue tasks (IEEE 1364-2005 17.6): $q_initialize, $q_add, $q_remove,
// $q_full and $q_exam with their status codes (0 done, 1 full, 2 no such queue, 3 empty,
// 4 bad type, 5 bad length, 6 id taken) and statistics in the module's time unit.
`timescale 1ns/1ps
module t;
  integer status, job, inform, v, full;
  integer fails = 0;
  task check(input bit c, input integer line);
    if (!c) begin
      $display("FAILED line %0d: status %0d job %0d inform %0d v %0d", line, status, job, inform, v);
      fails++;
    end
  endtask
  initial begin
    $q_initialize(5, 1, 3, status); check(status == 0, `__LINE__);
    $q_initialize(5, 2, 3, status); check(status == 6, `__LINE__);
    $q_initialize(6, 3, 3, status); check(status == 4, `__LINE__);
    $q_initialize(6, 1, 0, status); check(status == 5, `__LINE__);
    $q_initialize(7, 2, 2, status); check(status == 0, `__LINE__);
    job = 99;
    $q_remove(8, job, inform, status); check(status == 2 && job == 99, `__LINE__);
    full = $q_full(8, status); check(status == 2 && full == 0, `__LINE__);
    $q_remove(5, job, inform, status); check(status == 3, `__LINE__);
    // FIFO: arrivals at 0, 10, 30; removals at 35 and 41
    $q_add(5, 1, 10, status); check(status == 0, `__LINE__);
    #10 $q_add(5, 2, 11, status); check(status == 0, `__LINE__);
    #20 $q_add(5, 3, 12, status); check(status == 0 && $q_full(5, status) == 1, `__LINE__);
    $q_add(5, 4, 13, status); check(status == 1, `__LINE__);
    $q_exam(5, 5, v, status); check(status == 0 && v == 30, `__LINE__);
    #5 $q_remove(5, job, inform, status); check(status == 0 && job == 1 && inform == 10, `__LINE__);
    #6 $q_remove(5, job, inform, status); check(status == 0 && job == 2 && inform == 11, `__LINE__);
    $q_exam(5, 1, v, status); check(v == 1, `__LINE__);
    $q_exam(5, 2, v, status); check(v == 15, `__LINE__);
    $q_exam(5, 3, v, status); check(v == 3, `__LINE__);
    $q_exam(5, 4, v, status); check(v == 31, `__LINE__);
    $q_exam(5, 5, v, status); check(v == 11, `__LINE__);
    $q_exam(5, 6, v, status); check(v == 33, `__LINE__);
    // LIFO
    $q_add(7, 1, 1, status);
    $q_add(7, 1, 2, status);
    $q_remove(7, job, inform, status); check(status == 0 && inform == 2, `__LINE__);
    $q_remove(7, job, inform, status); check(status == 0 && inform == 1, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
