// expected
// join_none branches forked in a loop keep the automatic variables of their pass (IEEE
// 1800-2017 6.21, 9.3.2: each pass is an activation of the block); the loop variable of
// a for loop is one for all passes; a branch's write is seen by the parent.
class S;
  task dec(int d); #d; $display("%0t dec %0d", $time, d); endtask
  task body();
    for (int i = 1; i <= 3; i++) begin
      automatic int k = i;
      fork dec(k * 10); join_none
    end
    #100;
  endtask
endclass
module m;
  task automatic mdec(int d); #d; $display("%0t mdec %0d", $time, d); endtask
  initial begin
    S s = new;
    for (int i = 1; i <= 3; i++) begin
      automatic int k = i;
      fork mdec(k * 10); join_none
    end
    s.body();
  end
endmodule
module m2;
  initial begin
    for (int i = 0; i < 3; i++) fork $display("i %0d", i); join_none
    #1;
    for (int j = 0; j < 2; j++) begin
      automatic bit done = 0;
      automatic int v = j;
      fork begin #1 done = 1; $display("branch %0d", v); end join_none
      wait (done);
      $display("parent saw done %0d", j);
    end
  end
endmodule
