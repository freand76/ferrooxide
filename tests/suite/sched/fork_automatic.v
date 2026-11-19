// Automatic variables of a fork block (IEEE 1800-2017 9.3.2, 6.21): each started branch has
// the value the forking process gave them, even when the branch is started again before
// the earlier ones end (fork/join_none in a loop); automatic block variables and loop
// variables inside a branch are per thread too.
module test;
  reg clk = 0;
  always #5 clk = ~clk;
  integer sum = 0, fails = 0;
  integer seen[0:3];

  initial begin
    for (int i = 0; i < 4; i++) begin
      fork
        automatic int k = i;
        begin
          automatic int n = 0;
          for (int j = 0; j < 4 - k; j++) begin
            @(posedge clk);
            n++;
          end
          seen[k] = n;
          sum += k;
        end
      join_none
    end
    wait fork;
    for (int i = 0; i < 4; i++)
      if (seen[i] !== 4 - i) begin
        $display("FAILED thread %0d counted %0d", i, seen[i]);
        fails++;
      end
    if (sum != 6) begin
      $display("FAILED sum %0d", sum);
      fails++;
    end
    if (fails == 0) $display("PASSED");
    $finish;
  end
endmodule
