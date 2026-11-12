// fork/join, join_any, join_none, nested forks, variables shared between branches, a
// classic timeout pattern with disable of the fork block, disable of a task and of a block in
// another process, disable fork.
module tb;
  reg [7:0] a, b;
  integer n;
  reg done;
  task wait_and_say(input integer t, input [7:0] v);
    begin
      #t $display("%0t: task says %0d", $time, v);
    end
  endtask
  task long_task;
    begin
      #5 $display("%0t: long_task step 1", $time);
      #5 $display("%0t: long_task step 2 (should not print)", $time);
    end
  endtask
  initial begin
    a = 0; b = 0; n = 0; done = 0;
    fork
      #3 a = 1;
      #1 b = 2;
      begin #2 n = n + 1; #2 n = n + 1; end
    join
    $display("%0t: join: a=%0d b=%0d n=%0d", $time, a, b, n);
    fork
      #10 $display("%0t: slow branch", $time);
      #2 $display("%0t: fast branch", $time);
    join_any
    $display("%0t: after join_any", $time);
    fork
      #1 $display("%0t: join_none branch", $time);
    join_none
    $display("%0t: after join_none", $time);
    #20;
    fork
      wait_and_say(3, 33);
      wait_and_say(1, 11);
      fork
        #2 $display("%0t: nested fork 1", $time);
        #4 $display("%0t: nested fork 2", $time);
      join
    join
    $display("%0t: nested done", $time);
    // timeout pattern
    fork : guard
      begin #100 $display("%0t: TIMEOUT", $time); disable guard; end
      begin wait (done); $display("%0t: done seen", $time); disable guard; end
    join
    $display("%0t: after guard", $time);
    fork
      long_task;
      #7 disable long_task;
    join
    $display("%0t: after disable task", $time);
    fork
      #3 $display("%0t: never (disable fork)", $time);
      #4 $display("%0t: never either", $time);
    join_none
    #1 disable fork;
    #10 $display("%0t: end", $time);
    $finish;
  end
  initial begin
    #40 done = 1;
  end
  // disable a named block in another process
  initial begin : looper
    forever begin
      #6 $display("%0t: looper tick", $time);
    end
  end
  initial #20 disable looper;
endmodule
