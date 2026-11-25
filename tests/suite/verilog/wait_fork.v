// expected
// `wait fork` (IEEE 1800-2017 9.6.1): waits for the branches the process started (join_none
// and the rest of a join_any), not for those of other processes.
module test;
  reg [4:1] res = 0;
  initial begin
    fork
      #3 res[3] = 1;
      #4 res[4] = 1;
    join_none
    fork
      #1 res[1] = 1;
      #2 res[2] = 1;
    join_any
    $display("%0t after join_any %b", $time, res);
    wait fork;
    $display("%0t after wait fork %b", $time, res);
    wait fork;     // nothing left
    $display("%0t again %b", $time, res);
  end
  initial begin
    fork
      #10 $display("%0t other process", $time);
    join_none
    #5 wait fork;
    $display("%0t second process done", $time);
  end
endmodule
