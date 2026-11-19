// Parameters of type string (IEEE 1800-2017 6.20.2, 6.16): string methods, assignment to
// string variables, comparisons in constant expressions (generate) and at run time,
// overrides, printing.
module sub #(parameter string NAME = "def", parameter string OTHER = "abc") (output int len);
  initial len = NAME.len();
  if (NAME == "def") begin : d
    initial $display("default name");
  end else begin : o
    initial $display("name %s", NAME);
  end
endmodule

module t;
  parameter string S = "1234";
  parameter string A = "AB", B = "B";
  integer errors = 0;
  string s;
  int l1, l2;
  sub u1(.len(l1));
  sub #(.NAME("hello")) u2(.len(l2));
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, S.len(), 4);
    check(`__LINE__, S.atoi(), 1234);
    check(`__LINE__, S[1], "2");
    s = S;
    check(`__LINE__, s == "1234", 1);
    s = {S, "5"};
    check(`__LINE__, s.len(), 5);
    // lexicographic, not by value: "AB" < "B"
    check(`__LINE__, A < B, 1);
    check(`__LINE__, A == "AB", 1);
    check(`__LINE__, l1, 3);
    check(`__LINE__, l2, 5);
    if (errors == 0) $display("PASSED");
  end
endmodule
