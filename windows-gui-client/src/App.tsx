import React, {
    useState,
    useEffect,
    useRef,
    useLayoutEffect,
    useCallback,
} from "react";
import {
    AppBar,
    Toolbar,
    Typography,
    Button,
    Dialog,
    DialogActions,
    DialogContent,
    DialogContentText,
    makeStyles,
} from "@material-ui/core";

import {
    run,
    stop,
    subscribeLog,
    getStatus,
    openFile,
    exit,
    hide,
} from "./api";
import ShowInterfaceState from "./ShowInterfaceState";
import { InterfaceState } from "./InterfaceState";

const useStyles = makeStyles((theme) => ({
    root: {
        display: "flex",
        flexDirection: "column",
        height: "100%",
    },
    menuButton: {
        marginRight: theme.spacing(2),
    },
    title: {
        flexGrow: 1,
    },
    status: {
        flexGrow: 1,
        overflow: "auto",
        padding: theme.spacing(2),
    },
    showLogs: {
        flexGrow: 1,
        overflow: "auto",
        padding: theme.spacing(2),
        "& pre": {
            fontSize: "small",
            fontFamily: "Consolas , monospace",
            margin: ".3em 0 0 0",
            padding: 0,
        },
    },
    lastLogLine: {
        flexShrink: 0,
        fontSize: "small",
        whiteSpace: "nowrap",
        overflow: "hidden",
        fontFamily: "Consolas, monospace",
        margin: theme.spacing(1),
        padding: theme.spacing(1),
        backgroundColor: "#7487f1",
    },
    smallCaps: {
        fontVariant: "small-caps",
        textTransform: "none",
        fontFeatureSettings: "normal",
    },
}));

const ShowLogs: React.FC<{ logLines: string[]; className: string }> = ({
    logLines,
    className,
}) => {
    const divRef = useRef<HTMLDivElement>(null);

    useLayoutEffect(() => {
        if (divRef.current) {
            const el = divRef.current;
            el.scrollTop = el.scrollHeight - el.clientHeight;
        }
    }, []);

    useLayoutEffect(() => {
        if (divRef.current) {
            const el = divRef.current;
            if (el.scrollHeight - el.scrollTop - el.clientHeight <= 40) {
                el.scrollTop = el.scrollHeight - el.clientHeight;
            }
        }
    }, [logLines]);

    return (
        <div className={className} ref={divRef}>
            {logLines.map((l) => (
                <pre key={l}>{l}</pre>
            ))}
        </div>
    );
};

const App: React.FC = () => {
    const classes = useStyles();

    const [running, setRunning] = useState(false);
    const [busy, setBusy] = useState(false);
    const [interfaceState, setInterfaceState] = useState<null | InterfaceState>(
        null
    );
    const [lastLogLine, setLastLogLine] = useState("");
    const [openLogs, setOpenLogs] = useState(false);
    const [logLines, setLogLines] = useState<string[]>([]);
    const [getStatusInterval, setGetStatusInterval] = useState<number>(0);
    const [openConfirmExit, setOpenConfirmExit] = useState(false);

    // Initial loading.
    useEffect(() => {
        getStatus()
            .then((status) => {
                if (status != null) {
                    setInterfaceState(status);
                    setRunning(true);
                    setGetStatusInterval(
                        window.setInterval(async () => {
                            try {
                                setInterfaceState(await getStatus());
                            } catch (e) {
                                console.error(e);
                            }
                        }, 1000)
                    );
                }
            })
            .catch((e) => console.error(e));
    }, []);

    useEffect(() => {
        subscribeLog((logLine) => {
            console.info(logLine);
            setLogLines((old) => {
                if (old.length > 1024) {
                    return [...old.slice(256), logLine];
                }
                return [...old, logLine];
            });
            setLastLogLine(logLine);
        });
    }, []);

    const handleRunOrStopButtonClick = useCallback(async () => {
        setBusy(true);
        try {
            if (running) {
                await stop();
                setRunning(false);
                clearInterval(getStatusInterval);
                setLastLogLine("");
            } else {
                const fileName = await openFile();
                if (!fileName) {
                    return;
                }

                setLogLines([]);
                await run(fileName);
                setRunning(true);
                setGetStatusInterval(
                    window.setInterval(async () => {
                        try {
                            setInterfaceState(await getStatus());
                        } catch (e) {
                            console.error(e);
                        }
                    }, 1000)
                );
            }
        } catch (e) {
            console.error(e);
        } finally {
            setBusy(false);
        }
    }, [running, getStatusInterval]);

    // Shortcut keys.
    useEffect(() => {
        const onKeyDown = (event: KeyboardEvent) => {
            console.debug(event);
            if (event.target !== document.body) {
                return;
            }
            switch (event.key) {
                case "q":
                case "Q":
                    setOpenConfirmExit(true);
                    break;
                case "Escape":
                    hide();
                    break;
                case " ":
                case "Enter":
                    handleRunOrStopButtonClick();
                    break;
            }
        };
        document.addEventListener("keydown", onKeyDown);
        return () => document.removeEventListener("keydown", onKeyDown);
    }, [handleRunOrStopButtonClick]);

    return (
        <div className={classes.root}>
            <AppBar position="static">
                <Toolbar>
                    <Typography variant="h6" className={classes.title}>
                        TiTun
                    </Typography>
                    <Button
                        className={classes.smallCaps}
                        color="inherit"
                        disabled={busy}
                        onClick={handleRunOrStopButtonClick}
                    >
                        {running ? "Stop" : "Run"}
                    </Button>
                    <Button
                        className={classes.smallCaps}
                        color="inherit"
                        onClick={() => setOpenConfirmExit(true)}
                    >
                        Exit
                    </Button>
                </Toolbar>
            </AppBar>
            <Dialog
                open={openConfirmExit}
                onClose={() => setOpenConfirmExit(false)}
            >
                <DialogContent>
                    <DialogContentText>Exit TiTun?</DialogContentText>
                </DialogContent>
                <DialogActions>
                    <Button
                        color="primary"
                        onClick={() => setOpenConfirmExit(false)}
                    >
                        No
                    </Button>
                    <Button color="primary" onClick={() => exit()} autoFocus>
                        Yes
                    </Button>
                </DialogActions>
            </Dialog>
            <Dialog
                fullScreen
                open={openLogs}
                onClose={() => setOpenLogs(false)}
            >
                <AppBar position="static">
                    <Toolbar>
                        <Typography variant="h6" className={classes.title}>
                            Logs
                        </Typography>
                        <Button
                            color="inherit"
                            className={classes.smallCaps}
                            disabled={busy}
                            onClick={handleRunOrStopButtonClick}
                        >
                            {running ? "Stop" : "Run"}
                        </Button>
                        <Button
                            color="inherit"
                            className={classes.smallCaps}
                            onClick={() => setOpenLogs(false)}
                        >
                            Close
                        </Button>
                    </Toolbar>
                </AppBar>
                <ShowLogs logLines={logLines} className={classes.showLogs} />
            </Dialog>
            <div className={classes.status}>
                {running ? (
                    interfaceState ? (
                        <ShowInterfaceState interfaceState={interfaceState} />
                    ) : (
                        "Loading interface status..."
                    )
                ) : undefined}
            </div>
            <div
                className={classes.lastLogLine}
                onClick={() => setOpenLogs(true)}
            >
                {lastLogLine || <br></br>}
            </div>
        </div>
    );
};

export default App;
