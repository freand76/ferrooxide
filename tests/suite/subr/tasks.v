// Tasks (IEEE 1800 13.3): timing inside tasks, output arguments copied at the end, automatic
// tasks running concurrently, disable of a task and of a named block, static task variables
// shared between callers
module t;
  integer errors = 0;
  reg [7:0] out1, out2;
  integer log [0:7];
  integer n = 0;
  task automatic delayed(input integer d, input [7:0] v, output [7:0] o);
    #d o = v;
  endtask
  task note(input integer v);
    log[n] = v;
    n = n + 1;
  endtask
  task long_one;
    begin : body
      #5 note(1);
      #5 note(2);
    end
  endtask
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    out1 = 0;
    fork
      delayed(3, 8'd11, out1);
      delayed(1, 8'd22, out2);
    join
    check(`__LINE__, $time, 3);
    check(`__LINE__, out1, 11);
    check(`__LINE__, out2, 22);
    fork
      long_one;
      #7 disable long_one;          // ends it before the second note
    join
    check(`__LINE__, n, 1);
    check(`__LINE__, log[0], 1);
    begin : outer
      note(3);
      disable outer;
      note(4);
    end
    check(`__LINE__, n, 2);
    check(`__LINE__, log[1], 3);
    for (int i = 0; i < 5; i++) begin : loop
      if (i == 2) disable loop;     // like continue
      note(10 + i);
    end
    check(`__LINE__, n, 6);
    if (errors == 0) $display("PASSED");
  end
endmodule
