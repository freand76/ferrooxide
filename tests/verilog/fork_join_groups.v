// A join waits only for the branches of its own fork: branches left running by an earlier
// join_none or join_any do not count down a later join (IEEE 1800-2017 9.3.2).
module test;
  task automatic work(input int id, input int d);
    #d $display("%0t branch %0d done", $time, id);
  endtask
  initial begin
    fork
      work(1, 3);
      work(2, 20);
    join_none
    fork
      work(3, 5);
      work(4, 8);
    join_any
    $display("%0t join_any done", $time);
    fork
      work(5, 4);
      work(6, 6);
    join
    $display("%0t join done", $time);
    wait fork;
    $display("%0t wait fork done", $time);
  end
endmodule
