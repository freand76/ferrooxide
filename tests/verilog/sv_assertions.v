// Immediate assertions (assert / assume / cover, labels, default $error) and the
// messages of $error / $warning / $info / $fatal.
`timescale 1ns/1ps
module test;
  integer i = 1;
  reg x;

  task check(input integer v);
    assert (v == 1) else $warning("check: v is %0d", v);
  endtask

  initial begin
    assert (i == 1);
    assert (i == 0);
    assert (x);                         // x fails
    assert (i == 1) $display("pass 1"); else $display("fail 1");
    assume (i == 2) $display("pass 2"); else $display("fail 2");
    cover (i == 1) $display("covered");
    cover (i == 0) $display("not covered");
    named: assert (i == 3) else $error("named %0d", i);
    #2.5;
    check(1);
    check(7);
    $info("info at %0t", $realtime);
    begin : blk
      assert (i != 1);
    end
    $fatal(0, "fatal %s", "end");
    $display("not reached");
  end
endmodule
