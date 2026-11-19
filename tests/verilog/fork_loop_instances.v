// Fork branches started again while they still run (fork/join_none in a loop): each start
// is a thread of its own, with its own automatic task frame, loop counters and waits.
module test;
  reg clk = 0;
  always #5 clk = ~clk;
  integer i, total = 0;

  task automatic worker(input integer id, input integer n);
    integer k;
    for (k = 0; k < n; k = k + 1) @(posedge clk);
    total = total + id;
    $display("%0t worker %0d done after %0d edges", $time, id, n);
  endtask

  initial begin
    for (i = 1; i <= 4; i = i + 1) begin
      fork
        worker(i, 5 - i);
      join_none
      #1;
    end
    wait fork;
    $display("%0t all workers done, total %0d", $time, total);
    for (i = 0; i < 3; i = i + 1) begin
      fork
        begin
          #100 $display("%0t should not run", $time);
        end
      join_none
    end
    #12 disable fork;
    for (i = 0; i < 2; i = i + 1) begin
      fork
        worker(10 + i, 2 + i);
      join_none
      #3;
    end
    wait fork;
    for (i = 0; i < 3; i = i + 1) begin
      fork : blk
        begin
          #50 $display("%0t should not run either", $time);
        end
      join_none
    end
    #10 disable blk;
    #200 $display("%0t end", $time);
    $finish;
  end
endmodule
